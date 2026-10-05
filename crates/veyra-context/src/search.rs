//! Text search and file finding, built on ripgrep's `ignore` and `regex`
//! crates so `.gitignore` rules are honoured and large trees stay fast.

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepMatch {
    pub path: String,
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrepResult {
    pub matches: Vec<GrepMatch>,
    pub files_with_matches: usize,
    pub files_searched: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct GrepOptions {
    pub regex: bool,
    pub case_insensitive: bool,
    pub whole_word: bool,
    /// Glob restricting which files are searched (e.g. `*.rb`, `src/**`).
    pub glob: Option<String>,
    /// Sub-directory (relative to root) to search in.
    pub path: Option<String>,
    pub max_matches: usize,
    pub max_file_bytes: u64,
    /// Stop scanning further files once `max_matches` matches were found.
    /// Faster on huge trees, but which files are reported is not
    /// deterministic, so only heuristics (ranking) use it.
    pub stop_early: bool,
}

impl Default for GrepOptions {
    fn default() -> Self {
        Self {
            regex: true,
            case_insensitive: false,
            whole_word: false,
            glob: None,
            path: None,
            max_matches: 200,
            max_file_bytes: 2_000_000,
            stop_early: false,
        }
    }
}

const MAX_LINE_CHARS: usize = 240;

pub fn walk_files(root: &Path, sub: Option<&str>) -> Vec<PathBuf> {
    let base = match sub {
        Some(s) if !s.is_empty() && s != "." => root.join(s),
        _ => root.to_path_buf(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    ignore::WalkBuilder::new(&base)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            n != ".git" && n != "node_modules"
        })
        .build_parallel()
        .run(|| {
            let tx = tx.clone();
            Box::new(move |entry| {
                if let Ok(e) = entry {
                    if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                        let _ = tx.send(e.into_path());
                    }
                }
                ignore::WalkState::Continue
            })
        });
    drop(tx);
    let mut files: Vec<PathBuf> = rx.into_iter().collect();
    files.sort();
    files
}

pub fn rel_path(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn build_regex(pattern: &str, opts: &GrepOptions) -> Result<Regex, regex::Error> {
    let mut pat = if opts.regex {
        pattern.to_string()
    } else {
        regex::escape(pattern)
    };
    if opts.whole_word {
        // Only anchor ends that are word characters: `\b` after `)` would
        // never match `foo()` followed by `;`.
        let is_word = |c: Option<char>| c.map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
        let (start, end) = if opts.regex {
            (true, true)
        } else {
            (
                is_word(pattern.chars().next()),
                is_word(pattern.chars().last()),
            )
        };
        pat = format!(
            "{}(?:{pat}){}",
            if start { r"\b" } else { "" },
            if end { r"\b" } else { "" }
        );
    }
    RegexBuilder::new(&pat)
        .case_insensitive(opts.case_insensitive)
        .size_limit(10 * 1024 * 1024)
        .build()
}

pub fn grep(root: &Path, pattern: &str, opts: &GrepOptions) -> Result<GrepResult, regex::Error> {
    let re = build_regex(pattern, opts)?;
    let glob = opts
        .glob
        .as_deref()
        .and_then(|g| globset::Glob::new(g).ok())
        .map(|g| g.compile_matcher());
    let files = walk_files(root, opts.path.as_deref());
    let files: Vec<PathBuf> = files
        .into_iter()
        .filter(|p| {
            let rel = rel_path(root, p);
            match &glob {
                Some(g) => g.is_match(&rel) || g.is_match(p.file_name().unwrap_or_default()),
                None => true,
            }
        })
        .collect();
    let searched = files.len();

    let found = std::sync::atomic::AtomicUsize::new(0);
    let mut per_file: Vec<(String, Vec<GrepMatch>, bool)> = files
        .par_iter()
        .filter_map(|p| {
            if opts.stop_early
                && found.load(std::sync::atomic::Ordering::Relaxed) >= opts.max_matches
            {
                return None;
            }
            let md = std::fs::metadata(p).ok()?;
            if md.len() > opts.max_file_bytes {
                return None;
            }
            let bytes = std::fs::read(p).ok()?;
            if crate::looks_binary(&bytes) {
                return None;
            }
            let text = String::from_utf8_lossy(&bytes);
            let rel = rel_path(root, p);
            let mut ms = Vec::new();
            let mut capped = false;
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    if ms.len() >= opts.max_matches {
                        capped = true;
                        break;
                    }
                    ms.push(GrepMatch {
                        path: rel.clone(),
                        line: i as u32 + 1,
                        text: clip(line.trim_end(), MAX_LINE_CHARS),
                    });
                }
            }
            found.fetch_add(ms.len(), std::sync::atomic::Ordering::Relaxed);
            (!ms.is_empty()).then_some((rel, ms, capped))
        })
        .collect();
    per_file.sort_by(|a, b| a.0.cmp(&b.0));

    let mut result = GrepResult {
        files_with_matches: per_file.len(),
        files_searched: searched,
        ..Default::default()
    };
    'outer: for (_, ms, capped) in per_file {
        result.truncated |= capped;
        for m in ms {
            if result.matches.len() >= opts.max_matches {
                result.truncated = true;
                break 'outer;
            }
            result.matches.push(m);
        }
    }
    Ok(result)
}

/// Find files by glob (`**/*_spec.rb`) or, when the pattern has no glob
/// characters, by case-insensitive substring of the relative path.
pub fn find_files(root: &Path, pattern: &str, limit: usize) -> (Vec<String>, bool) {
    let files = walk_files(root, None);
    let is_glob = pattern.contains(['*', '?', '[', '{']);
    let matcher = if is_glob {
        globset::GlobBuilder::new(pattern)
            .literal_separator(false)
            .build()
            .ok()
            .map(|g| g.compile_matcher())
    } else {
        None
    };
    let needle = pattern.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut truncated = false;
    for p in files {
        let rel = rel_path(root, &p);
        let hit = match &matcher {
            Some(m) => m.is_match(&rel) || m.is_match(p.file_name().unwrap_or_default()),
            None => rel.to_ascii_lowercase().contains(&needle),
        };
        if hit {
            if out.len() >= limit {
                truncated = true;
                break;
            }
            out.push(rel);
        }
    }
    (out, truncated)
}

pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let w = |rel: &str, t: &[u8]| {
            let p = d.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "src/auth.rs",
            b"fn authenticate() {}\nfn other() { authenticate(); }\n",
        );
        w("src/lib.rs", b"mod auth;\n");
        w("node_modules/x/index.js", b"authenticate\n");
        w("bin/blob", b"authenticate\0\0");
        w(".gitignore", b"dist/\n");
        w("dist/out.js", b"authenticate\n");
        std::fs::create_dir_all(d.path().join(".git")).unwrap();
        d
    }

    #[test]
    fn grep_respects_ignores_and_skips_binary() {
        let d = repo();
        let r = grep(d.path(), "authenticate", &GrepOptions::default()).unwrap();
        let paths: Vec<_> = r
            .matches
            .iter()
            .map(|m| (m.path.as_str(), m.line))
            .collect();
        assert_eq!(paths, vec![("src/auth.rs", 1), ("src/auth.rs", 2)]);
        assert_eq!(r.files_with_matches, 1);
    }

    #[test]
    fn grep_literal_whole_word_and_limits() {
        let d = repo();
        let opts = GrepOptions {
            regex: false,
            whole_word: true,
            max_matches: 1,
            ..Default::default()
        };
        let r = grep(d.path(), "authenticate()", &opts).unwrap();
        assert_eq!(r.matches.len(), 1);
        assert!(r.truncated);
        assert!(grep(d.path(), "(unclosed", &GrepOptions::default()).is_err());
    }

    #[test]
    fn find_files_by_glob_and_substring() {
        let d = repo();
        assert_eq!(
            find_files(d.path(), "*.rs", 10).0,
            vec!["src/auth.rs", "src/lib.rs"]
        );
        assert_eq!(find_files(d.path(), "AUTH", 10).0, vec!["src/auth.rs"]);
        let (f, t) = find_files(d.path(), "src", 1);
        assert_eq!(f.len(), 1);
        assert!(t);
    }
}
