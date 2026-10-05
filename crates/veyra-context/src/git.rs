//! Thin wrappers around the `git` CLI. Only read-only operations live here;
//! mutations go through the tool layer and its permission checks.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEntry {
    pub path: String,
    /// Two-letter porcelain code, e.g. ` M`, `??`, `A `.
    pub code: String,
    pub orig_path: Option<String>,
}

impl StatusEntry {
    pub fn untracked(&self) -> bool {
        self.code == "??"
    }
}

pub fn git(root: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=off", "-c", "color.ui=never"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn is_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// Top-level directory of the repository containing `dir`.
pub fn toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| std::path::PathBuf::from(s.trim()))
}

pub fn head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn branch(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
}

/// `git status --porcelain=v1 -z`, parsed.
pub fn status(root: &Path) -> anyhow::Result<Vec<StatusEntry>> {
    let raw = git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    Ok(parse_porcelain_z(&raw))
}

pub fn parse_porcelain_z(raw: &str) -> Vec<StatusEntry> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0').filter(|s| !s.is_empty());
    while let Some(rec) = parts.next() {
        if rec.len() < 4 {
            continue;
        }
        let code = rec[..2].to_string();
        let path = rec[3..].to_string();
        let orig_path = if code.starts_with('R') || code.starts_with('C') {
            parts.next().map(str::to_string)
        } else {
            None
        };
        out.push(StatusEntry {
            path,
            code,
            orig_path,
        });
    }
    out
}

/// Paths with uncommitted changes (including untracked files).
pub fn dirty_paths(root: &Path) -> Vec<String> {
    status(root)
        .map(|v| v.into_iter().map(|e| e.path).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parsing() {
        let raw = " M src/a.rs\0?? new file.txt\0R  new.rs\0old.rs\0A  b.rs\0";
        let e = parse_porcelain_z(raw);
        assert_eq!(e.len(), 4);
        assert_eq!(e[0].path, "src/a.rs");
        assert_eq!(e[0].code, " M");
        assert!(e[1].untracked());
        assert_eq!(e[1].path, "new file.txt");
        assert_eq!(e[2].path, "new.rs");
        assert_eq!(e[2].orig_path.as_deref(), Some("old.rs"));
        assert_eq!(e[3].code, "A ");
    }
}
