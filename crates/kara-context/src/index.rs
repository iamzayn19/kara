//! Incremental repository index stored in SQLite.
//!
//! The index lives in `~/.kara/cache/index-<hash of root>.db`, never inside
//! the repository. On refresh, files whose size and mtime are unchanged are
//! not re-read; only new or modified files are parsed (in parallel).

use crate::languages::LanguageRegistry;
use crate::symbols::{PatternExtractor, SymbolExtractor};
use rayon::prelude::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

const SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexStats {
    pub files: usize,
    pub parsed: usize,
    pub removed: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoStats {
    pub files: usize,
    pub lines: u64,
    pub bytes: u64,
    pub symbols: usize,
    pub test_files: usize,
    pub by_language: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolHit {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone)]
pub struct RepoIndex {
    root: PathBuf,
    db_path: PathBuf,
    registry: LanguageRegistry,
    max_file_bytes: u64,
}

struct Scanned {
    rel: String,
    size: u64,
    mtime: i64,
}

impl RepoIndex {
    /// Open (or create) the index for `root`, stored in `cache_dir`.
    pub fn open(
        root: &Path,
        cache_dir: &Path,
        registry: LanguageRegistry,
        max_file_bytes: u64,
    ) -> anyhow::Result<RepoIndex> {
        std::fs::create_dir_all(cache_dir)?;
        let mut h = Sha256::new();
        h.update(root.to_string_lossy().as_bytes());
        let name = format!("index-{}.db", &hex::encode(h.finalize())[..16]);
        let idx = RepoIndex {
            root: root.to_path_buf(),
            db_path: cache_dir.join(name),
            registry,
            max_file_bytes,
        };
        idx.init()?;
        Ok(idx)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn registry(&self) -> &LanguageRegistry {
        &self.registry
    }

    fn conn(&self) -> anyhow::Result<Connection> {
        let c = Connection::open(&self.db_path)?;
        c.busy_timeout(std::time::Duration::from_secs(10))?;
        c.pragma_update(None, "journal_mode", "WAL")?;
        c.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(c)
    }

    fn init(&self) -> anyhow::Result<()> {
        let c = self.conn()?;
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS files (
                path TEXT PRIMARY KEY, lang TEXT, size INTEGER NOT NULL, mtime INTEGER NOT NULL,
                lines INTEGER NOT NULL DEFAULT 0, is_test INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS symbols (
                path TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL, line INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name COLLATE NOCASE);
             CREATE INDEX IF NOT EXISTS symbols_path ON symbols(path);
             CREATE TABLE IF NOT EXISTS imports (path TEXT NOT NULL, target TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS imports_path ON imports(path);
             CREATE INDEX IF NOT EXISTS imports_target ON imports(target);",
        )?;
        // Rebuild when the schema or the language packs change.
        let fingerprint = format!("{SCHEMA_VERSION}:{}", self.packs_fingerprint());
        let stored: Option<String> = c
            .query_row("SELECT value FROM meta WHERE key='fingerprint'", [], |r| {
                r.get(0)
            })
            .optional()?;
        if stored.as_deref() != Some(&fingerprint) {
            c.execute_batch("DELETE FROM files; DELETE FROM symbols; DELETE FROM imports;")?;
            c.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES('fingerprint', ?1)",
                [fingerprint],
            )?;
        }
        Ok(())
    }

    fn packs_fingerprint(&self) -> String {
        let mut h = Sha256::new();
        for p in self.registry.packs() {
            h.update(p.def.id.as_bytes());
            for r in &p.def.symbols {
                h.update(r.pattern.as_bytes());
            }
            for r in &p.def.imports.patterns {
                h.update(r.as_bytes());
            }
            for t in &p.def.test_patterns {
                h.update(t.as_bytes());
            }
        }
        hex::encode(h.finalize())[..16].to_string()
    }

    fn scan(&self) -> Vec<Scanned> {
        let (tx, rx) = std::sync::mpsc::channel();
        let root = self.root.clone();
        ignore::WalkBuilder::new(&self.root)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .filter_entry(|e| {
                let n = e.file_name().to_string_lossy();
                !(n == ".git"
                    || n == "node_modules"
                    || n == ".kara"
                    || n == "target" && e.depth() == 1)
            })
            .build_parallel()
            .run(|| {
                let tx = tx.clone();
                let root = root.clone();
                Box::new(move |entry| {
                    if let Ok(e) = entry {
                        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                            if let Ok(md) = e.metadata() {
                                let rel = e
                                    .path()
                                    .strip_prefix(&root)
                                    .unwrap_or(e.path())
                                    .components()
                                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                                    .collect::<Vec<_>>()
                                    .join("/");
                                let mtime = md
                                    .modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                    .map(|d| d.as_nanos() as i64)
                                    .unwrap_or(0);
                                let _ = tx.send(Scanned {
                                    rel,
                                    size: md.len(),
                                    mtime,
                                });
                            }
                        }
                    }
                    ignore::WalkState::Continue
                })
            });
        drop(tx);
        rx.into_iter().collect()
    }

    /// Bring the index up to date. Unchanged files are skipped.
    pub fn refresh(&self) -> anyhow::Result<IndexStats> {
        let start = Instant::now();
        let scanned = self.scan();
        let mut c = self.conn()?;

        let mut known: HashMap<String, (u64, i64)> = HashMap::new();
        {
            let mut st = c.prepare("SELECT path, size, mtime FROM files")?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (p, s, m) = row?;
                known.insert(p, (s, m));
            }
        }

        let changed: Vec<&Scanned> = scanned
            .iter()
            .filter(|s| known.get(&s.rel) != Some(&(s.size, s.mtime)))
            .collect();

        let extractor = PatternExtractor;
        let parsed: Vec<_> = changed
            .par_iter()
            .map(|s| {
                let pack = self.registry.for_path(&s.rel);
                let is_test = pack.map(|p| p.is_test_file(&s.rel)).unwrap_or(false);
                let mut extracted = Default::default();
                if let Some(pack) = pack {
                    if s.size <= self.max_file_bytes {
                        if let Ok(bytes) = std::fs::read(self.root.join(&s.rel)) {
                            if !crate::looks_binary(&bytes) {
                                let text = String::from_utf8_lossy(&bytes);
                                extracted = extractor.extract(pack, &text);
                            }
                        }
                    }
                }
                (*s, pack.map(|p| p.id().to_string()), is_test, extracted)
            })
            .collect();

        let present: std::collections::HashSet<&str> =
            scanned.iter().map(|s| s.rel.as_str()).collect();
        let removed: Vec<String> = known
            .keys()
            .filter(|k| !present.contains(k.as_str()))
            .cloned()
            .collect();

        let tx = c.transaction()?;
        {
            let mut del_file = tx.prepare("DELETE FROM files WHERE path = ?1")?;
            let mut del_sym = tx.prepare("DELETE FROM symbols WHERE path = ?1")?;
            let mut del_imp = tx.prepare("DELETE FROM imports WHERE path = ?1")?;
            for r in &removed {
                del_file.execute([r])?;
                del_sym.execute([r])?;
                del_imp.execute([r])?;
            }
            let mut ins_file = tx.prepare(
                "INSERT OR REPLACE INTO files(path, lang, size, mtime, lines, is_test) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut ins_sym =
                tx.prepare("INSERT INTO symbols(path, name, kind, line) VALUES (?1, ?2, ?3, ?4)")?;
            let mut ins_imp = tx.prepare("INSERT INTO imports(path, target) VALUES (?1, ?2)")?;
            for (s, lang, is_test, ex) in &parsed {
                del_sym.execute([&s.rel])?;
                del_imp.execute([&s.rel])?;
                ins_file.execute(params![
                    s.rel,
                    lang,
                    s.size as i64,
                    s.mtime,
                    ex.lines as i64,
                    *is_test as i64
                ])?;
                for sym in &ex.symbols {
                    ins_sym.execute(params![s.rel, sym.name, sym.kind, sym.line as i64])?;
                }
                for imp in &ex.imports {
                    ins_imp.execute(params![s.rel, imp])?;
                }
            }
        }
        tx.commit()?;

        Ok(IndexStats {
            files: scanned.len(),
            parsed: parsed.len(),
            removed: removed.len(),
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Find symbol definitions. Exact (case-insensitive) matches first, then
    /// prefix, then substring.
    pub fn find_symbol(&self, name: &str, limit: usize) -> anyhow::Result<Vec<SymbolHit>> {
        let c = self.conn()?;
        let mut st = c.prepare(
            "SELECT name, kind, path, line,
                CASE WHEN name = ?1 THEN 0 WHEN name = ?1 COLLATE NOCASE THEN 1
                     WHEN name LIKE ?2 ESCAPE '\\' THEN 2 ELSE 3 END AS rank
             FROM symbols
             WHERE name LIKE ?3 ESCAPE '\\'
             ORDER BY rank, length(name), path, line
             LIMIT ?4",
        )?;
        let esc = escape_like(name);
        let rows = st.query_map(
            params![name, format!("{esc}%"), format!("%{esc}%"), limit as i64],
            |r| {
                Ok(SymbolHit {
                    name: r.get(0)?,
                    kind: r.get(1)?,
                    path: r.get(2)?,
                    line: r.get::<_, i64>(3)? as u32,
                })
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn symbols_in(&self, path: &str) -> anyhow::Result<Vec<SymbolHit>> {
        let c = self.conn()?;
        let mut st =
            c.prepare("SELECT name, kind, path, line FROM symbols WHERE path = ?1 ORDER BY line")?;
        let rows = st.query_map([path], |r| {
            Ok(SymbolHit {
                name: r.get(0)?,
                kind: r.get(1)?,
                path: r.get(2)?,
                line: r.get::<_, i64>(3)? as u32,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Files whose import targets mention `needle` (a module or file stem).
    pub fn importers_of(&self, needle: &str, limit: usize) -> anyhow::Result<Vec<String>> {
        let c = self.conn()?;
        let mut st = c.prepare(
            "SELECT DISTINCT path FROM imports WHERE target LIKE ?1 ESCAPE '\\' LIMIT ?2",
        )?;
        let rows = st.query_map(
            params![format!("%{}%", escape_like(needle)), limit as i64],
            |r| r.get::<_, String>(0),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn imports_of(&self, path: &str) -> anyhow::Result<Vec<String>> {
        let c = self.conn()?;
        let mut st = c.prepare("SELECT target FROM imports WHERE path = ?1")?;
        let rows = st.query_map([path], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// All indexed file paths with (lang, is_test).
    pub fn files(&self) -> anyhow::Result<Vec<(String, Option<String>, bool)>> {
        let c = self.conn()?;
        let mut st = c.prepare("SELECT path, lang, is_test FROM files ORDER BY path")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn stats(&self) -> anyhow::Result<RepoStats> {
        let c = self.conn()?;
        let mut s = RepoStats::default();
        let (files, lines, bytes, tests): (i64, i64, i64, i64) = c.query_row(
            "SELECT COUNT(*), COALESCE(SUM(lines),0), COALESCE(SUM(size),0), COALESCE(SUM(is_test),0) FROM files",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        s.files = files as usize;
        s.lines = lines as u64;
        s.bytes = bytes as u64;
        s.test_files = tests as usize;
        s.symbols =
            c.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get::<_, i64>(0))? as usize;
        let mut st = c.prepare(
            "SELECT lang, COUNT(*) FROM files WHERE lang IS NOT NULL GROUP BY lang ORDER BY COUNT(*) DESC",
        )?;
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (l, n) = row?;
            s.by_language.insert(l, n as usize);
        }
        Ok(s)
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn incremental_refresh() {
        let repo = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            "lib/auth.rb",
            "class Auth\n  def login(u)\n  end\nend\n",
        );
        write(
            repo.path(),
            "spec/auth_spec.rb",
            "require 'auth'\ndescribe Auth do\nend\n",
        );
        write(repo.path(), "README.md", "# hi\n");
        write(repo.path(), ".gitignore", "tmp/\n");
        write(repo.path(), "tmp/ignored.rb", "class Ignored; end\n");
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();

        let idx = RepoIndex::open(
            repo.path(),
            cache.path(),
            LanguageRegistry::builtin(),
            1_000_000,
        )
        .unwrap();
        let s1 = idx.refresh().unwrap();
        assert_eq!(s1.files, 4, "{s1:?}");
        assert_eq!(s1.parsed, 4);

        let s2 = idx.refresh().unwrap();
        assert_eq!(s2.parsed, 0, "unchanged files are not reparsed");

        std::thread::sleep(std::time::Duration::from_millis(20));
        write(
            repo.path(),
            "lib/auth.rb",
            "class Auth\n  def login(u)\n  end\n  def logout\n  end\nend\n",
        );
        std::fs::remove_file(repo.path().join("README.md")).unwrap();
        let s3 = idx.refresh().unwrap();
        assert_eq!(s3.parsed, 1);
        assert_eq!(s3.removed, 1);

        let hits = idx.find_symbol("logout", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "lib/auth.rb");
        assert_eq!(hits[0].line, 4);
        assert!(idx.find_symbol("Ignored", 10).unwrap().is_empty());

        let stats = idx.stats().unwrap();
        assert_eq!(stats.test_files, 1);
        assert_eq!(stats.by_language.get("ruby"), Some(&2));
        assert_eq!(
            idx.importers_of("auth", 10).unwrap(),
            vec!["spec/auth_spec.rb"]
        );
    }

    #[test]
    fn symbol_ranking_and_like_escaping() {
        let repo = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        write(
            repo.path(),
            "a.py",
            "def paginate():\n    pass\ndef paginate_users():\n    pass\ndef do_paginate():\n    pass\ndef a_b():\n    pass\ndef axb():\n    pass\n",
        );
        let idx = RepoIndex::open(
            repo.path(),
            cache.path(),
            LanguageRegistry::builtin(),
            1_000_000,
        )
        .unwrap();
        idx.refresh().unwrap();
        let names: Vec<String> = idx
            .find_symbol("paginate", 10)
            .unwrap()
            .into_iter()
            .map(|h| h.name)
            .collect();
        assert_eq!(names, vec!["paginate", "paginate_users", "do_paginate"]);
        let names: Vec<String> = idx
            .find_symbol("a_b", 10)
            .unwrap()
            .into_iter()
            .map(|h| h.name)
            .collect();
        assert_eq!(
            names,
            vec!["a_b"],
            "underscore is literal, not a LIKE wildcard"
        );
    }

    #[test]
    fn huge_and_binary_files_are_metadata_only() {
        let repo = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        std::fs::write(
            repo.path().join("big.py"),
            "def f():\n    pass\n".repeat(1000),
        )
        .unwrap();
        std::fs::write(repo.path().join("bin.py"), b"def g():\0\0\0").unwrap();
        let idx = RepoIndex::open(
            repo.path(),
            cache.path(),
            LanguageRegistry::builtin(),
            1_000,
        )
        .unwrap();
        let s = idx.refresh().unwrap();
        assert_eq!(s.files, 2);
        assert!(idx.find_symbol("f", 10).unwrap().is_empty());
        assert!(idx.find_symbol("g", 10).unwrap().is_empty());
    }
}
