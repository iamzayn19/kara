//! Context ranking: given a task, pick the handful of files the agent should
//! look at first, with reasons and key symbol locations. Contents are never
//! included; the agent reads ranges on demand.
//!
//! Signals, strongest first: direct symbol matches, path matches, tests for
//! matched files, importers of matched files, uncommitted changes, and finally
//! a bounded lexical search when nothing else matched.

use crate::index::RepoIndex;
use crate::search::{grep, GrepOptions};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankedFile {
    pub path: String,
    pub score: f32,
    pub reasons: Vec<String>,
    /// (name, kind, line) of the most relevant symbols in the file.
    pub symbols: Vec<(String, String, u32)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Orientation {
    pub terms: Vec<String>,
    pub files: Vec<RankedFile>,
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "from",
    "into",
    "when",
    "what",
    "why",
    "how",
    "are",
    "was",
    "were",
    "will",
    "can",
    "could",
    "should",
    "would",
    "fix",
    "add",
    "make",
    "find",
    "explain",
    "please",
    "failing",
    "fails",
    "fail",
    "test",
    "tests",
    "spec",
    "specs",
    "code",
    "repository",
    "repo",
    "file",
    "files",
    "function",
    "method",
    "class",
    "bug",
    "issue",
    "work",
    "works",
    "does",
    "not",
    "all",
    "any",
    "use",
    "using",
    "implement",
    "support",
    "change",
    "update",
    "new",
    "get",
    "set",
    "there",
    "here",
    "about",
    "some",
    "them",
    "they",
    "its",
    "our",
    "your",
    "you",
    "have",
    "has",
    "had",
    "but",
    "out",
    "run",
    "check",
    "current",
    "slow",
    "feature",
    "described",
    "convert",
    "review",
    "build",
    "also",
    "just",
    "only",
    "then",
    "than",
    "which",
];

/// Extract search terms from a natural-language task.
pub fn terms(task: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: String| {
        if t.len() >= 3 && !STOPWORDS.contains(&t.as_str()) && !out.contains(&t) {
            out.push(t);
        }
    };
    for raw in
        task.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':' || c == '.' || c == '/'))
    {
        let raw = raw.trim_matches(|c: char| c == '.' || c == ':' || c == '/');
        if raw.is_empty() || raw.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // Keep identifiers that look like code verbatim (snake_case, CamelCase, paths).
        let codeish = raw.contains('_')
            || raw.contains("::")
            || raw.contains('/')
            || raw.contains('.')
            || raw.chars().skip(1).any(|c| c.is_uppercase());
        if codeish {
            push(raw.to_string());
        }
        for part in split_identifier(raw) {
            push(part.to_lowercase());
        }
    }
    out
}

fn split_identifier(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let boundary = c.is_uppercase()
            && !cur.is_empty()
            && (chars[i - 1].is_lowercase()
                || chars.get(i + 1).map(|n| n.is_lowercase()).unwrap_or(false)
                    && chars[i - 1].is_uppercase());
        if boundary {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Rough relatedness of a search term and an identifier / path segment.
fn term_score(term: &str, ident: &str) -> f32 {
    let t = term.to_lowercase();
    let i = ident.to_lowercase();
    if t == i {
        return 3.0;
    }
    if t.len() >= 4 && i.contains(&t) {
        return 2.0;
    }
    if i.len() >= 5 && t.contains(&i) {
        return 1.5;
    }
    let common = t.chars().zip(i.chars()).take_while(|(a, b)| a == b).count();
    if common >= 5 || (common >= 4 && common + 1 >= t.len().min(i.len())) {
        return 1.0;
    }
    0.0
}

type Scores = HashMap<String, (f32, BTreeSet<String>, Vec<(String, String, u32)>)>;

fn bump(scores: &mut Scores, path: &str, s: f32, reason: String) {
    let e = scores.entry(path.to_string()).or_default();
    e.0 += s;
    e.1.insert(reason);
}

pub fn orient(
    index: &RepoIndex,
    task: &str,
    changed: &[String],
    max_files: usize,
) -> anyhow::Result<Orientation> {
    let terms = terms(task);
    let mut scores: Scores = HashMap::new();

    let files = index.files()?;
    let test_files: Vec<&String> = files.iter().filter(|f| f.2).map(|f| &f.0).collect();

    // 1. Symbols.
    let mut symbol_sites: HashMap<String, Vec<(String, String, u32)>> = HashMap::new();
    for term in &terms {
        let probe = term.rsplit("::").next().unwrap_or(term);
        let probe = probe.rsplit('.').next().unwrap_or(probe);
        if probe.len() < 3 {
            continue;
        }
        for hit in index.find_symbol(probe, 40)? {
            let s = term_score(probe, &hit.name);
            if s <= 0.0 {
                continue;
            }
            let weight = if s >= 3.0 { 6.0 } else { 2.0 * s };
            bump(
                &mut scores,
                &hit.path,
                weight.min(8.0),
                format!("defines `{}`", hit.name),
            );
            symbol_sites.entry(hit.path.clone()).or_default().push((
                hit.name.clone(),
                hit.kind.clone(),
                hit.line,
            ));
        }
    }

    // 2. Paths. A path can only score if it shares a 4-character window
    // with a term (every branch of `term_score` implies that), so filter
    // cheaply before the full comparison. Keeps ranking fast on 100k files.
    let grams: Vec<String> = terms
        .iter()
        .flat_map(|t| {
            let chars: Vec<char> = t.to_lowercase().chars().collect();
            if chars.len() < 4 {
                vec![chars.iter().collect::<String>()]
            } else {
                chars.windows(4).map(|w| w.iter().collect::<String>()).collect()
            }
        })
        .collect();
    for (path, _, _) in &files {
        let lower = path.to_lowercase();
        if !grams.iter().any(|g| lower.contains(g.as_str())) {
            continue;
        }
        let segs: Vec<&str> = path
            .split(['/', '.', '_', '-'])
            .filter(|s| s.len() >= 3)
            .collect();
        let mut best = 0.0f32;
        let mut best_term = "";
        for term in &terms {
            if term.contains('/') && path.contains(term.as_str()) {
                best = best.max(6.0);
                best_term = term;
                continue;
            }
            for seg in &segs {
                let s = term_score(term, seg);
                if s > best {
                    best = s;
                    best_term = term;
                }
            }
        }
        if best >= 1.0 {
            bump(
                &mut scores,
                path,
                best * 1.5,
                format!("path matches `{best_term}`"),
            );
        }
    }

    // 3. Uncommitted changes.
    for c in changed {
        if files.iter().any(|f| &f.0 == c) {
            bump(&mut scores, c, 2.0, "uncommitted change".into());
        }
    }

    // 4. Lexical fallback when the index found little: one combined pass,
    // stopping early on huge trees.
    if scores.len() < 3 {
        let lexical: Vec<&String> = terms.iter().filter(|t| t.len() >= 4).take(3).collect();
        if !lexical.is_empty() {
            let pattern = lexical
                .iter()
                .map(|t| regex::escape(t))
                .collect::<Vec<_>>()
                .join("|");
            let opts = GrepOptions {
                regex: true,
                case_insensitive: true,
                max_matches: 120,
                stop_early: true,
                ..Default::default()
            };
            if let Ok(r) = grep(index.root(), &pattern, &opts) {
                let mut counts: HashMap<&str, (usize, &str)> = HashMap::new();
                for m in &r.matches {
                    let lower = m.text.to_lowercase();
                    let term = lexical
                        .iter()
                        .find(|t| lower.contains(&t.to_lowercase()))
                        .map(|t| t.as_str())
                        .unwrap_or("");
                    let e = counts.entry(m.path.as_str()).or_insert((0, term));
                    e.0 += 1;
                }
                for (p, (n, term)) in counts {
                    bump(
                        &mut scores,
                        p,
                        (n as f32).sqrt().min(3.0),
                        format!("mentions `{term}`"),
                    );
                }
            }
        }
    }

    // 5. Tests and importers of the strongest candidates.
    let mut top: Vec<(String, f32)> = scores.iter().map(|(p, v)| (p.clone(), v.0)).collect();
    top.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    for (path, score) in top.iter().take(5) {
        if test_files.contains(&path) {
            continue;
        }
        let stem = file_stem(path);
        if stem.len() < 3 {
            continue;
        }
        for t in &test_files {
            let tstem = file_stem(t);
            if tstem.contains(&stem) {
                bump(
                    &mut scores,
                    t,
                    (score * 0.6).max(2.0),
                    format!("tests {path}"),
                );
            }
        }
        for imp in index.importers_of(&stem, 8)? {
            if &imp != path {
                bump(&mut scores, &imp, 1.0, format!("imports {stem}"));
            }
        }
    }

    let mut ranked: Vec<RankedFile> = scores
        .into_iter()
        .map(|(path, (score, reasons, _))| {
            let mut symbols = symbol_sites.remove(&path).unwrap_or_default();
            symbols.sort_by_key(|s| s.2);
            symbols.dedup();
            symbols.truncate(6);
            RankedFile {
                path,
                score,
                reasons: reasons.into_iter().take(4).collect(),
                symbols,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path.cmp(&b.path))
    });
    ranked.truncate(max_files);
    Ok(Orientation {
        terms,
        files: ranked,
    })
}

fn file_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name).to_lowercase();
    stem.trim_start_matches("test_")
        .trim_end_matches("_test")
        .trim_end_matches("_spec")
        .trim_end_matches("test")
        .trim_end_matches("tests")
        .to_string()
}

impl Orientation {
    /// Compact text for the model.
    pub fn render(&self) -> String {
        if self.files.is_empty() {
            return "No strongly related files found by the index; use grep/find_files to explore.\n".into();
        }
        let mut s =
            String::from("Likely relevant files (ranked by Veyra's index; read before editing):\n");
        for f in &self.files {
            s.push_str(&format!("- {} ({})\n", f.path, f.reasons.join("; ")));
            for (name, kind, line) in &f.symbols {
                s.push_str(&format!("    {kind} {name} at line {line}\n"));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::LanguageRegistry;

    #[test]
    fn term_extraction() {
        let t = terms(
            "Fix the failing authentication specs in SessionStore#valid? and api/v2/users.rb",
        );
        assert!(t.contains(&"authentication".to_string()));
        assert!(t.contains(&"SessionStore".to_string()));
        assert!(t.contains(&"session".to_string()));
        assert!(t.contains(&"store".to_string()));
        assert!(t.contains(&"api/v2/users.rb".to_string()));
        assert!(!t.contains(&"the".to_string()));
        assert!(!t.contains(&"failing".to_string()));
        assert_eq!(
            split_identifier("HTTPServerError"),
            vec!["HTTP", "Server", "Error"]
        );
        assert_eq!(
            split_identifier("apply_discount"),
            vec!["apply", "discount"]
        );
    }

    #[test]
    fn ranks_symbol_and_test_files() {
        let repo = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let w = |rel: &str, t: &str| {
            let p = repo.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(
            "app/payments/charge.py",
            "def create_charge(amount):\n    pass\n",
        );
        w(
            "app/auth/session.py",
            "class SessionStore:\n    def authenticate(self, token):\n        pass\n",
        );
        w(
            "tests/test_session.py",
            "from app.auth.session import SessionStore\n",
        );
        w(
            "app/views.py",
            "from app.auth.session import SessionStore\n",
        );
        w("README.md", "docs\n");
        let idx = RepoIndex::open(
            repo.path(),
            cache.path(),
            LanguageRegistry::builtin(),
            1_000_000,
        )
        .unwrap();
        idx.refresh().unwrap();
        let o = orient(
            &idx,
            "Fix why authenticate rejects valid session tokens",
            &[],
            5,
        )
        .unwrap();
        let paths: Vec<&str> = o.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths[0], "app/auth/session.py", "{o:#?}");
        assert!(paths.contains(&"tests/test_session.py"), "{paths:?}");
        assert!(!paths.contains(&"app/payments/charge.py"), "{paths:?}");
        let top = &o.files[0];
        assert!(top
            .symbols
            .iter()
            .any(|s| s.0 == "authenticate" && s.2 == 2));
        assert!(o.render().contains("app/auth/session.py"));
    }
}
