//! Workspace path confinement.

use crate::secrets::is_secret_path;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathError {
    #[error("empty path")]
    Empty,
    #[error("path contains a NUL byte")]
    Nul,
    #[error("cannot resolve path {0}: {1}")]
    Resolve(String, String),
}

/// A path after lexical normalisation and symlink resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    /// Absolute, symlink-resolved path.
    pub abs: PathBuf,
    /// Path relative to the workspace root using `/` separators, when inside.
    pub rel: Option<String>,
    /// Whether `abs` lies inside the workspace root.
    pub inside: bool,
    /// Whether the path refers to credentials or other secrets.
    pub secret: bool,
    /// Whether the path is inside the repository's `.git` directory. Writes
    /// there could install hooks that execute code, so they are high risk.
    pub git_internal: bool,
}

impl ResolvedPath {
    /// Display form: relative when inside the workspace.
    pub fn display(&self) -> String {
        self.rel
            .clone()
            .unwrap_or_else(|| self.abs.display().to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    extra_readable: Vec<PathBuf>,
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = dunce_canonicalize(root.as_ref())?;
        Ok(Self {
            root,
            extra_readable: Vec::new(),
        })
    }

    pub fn with_extra_readable(mut self, paths: &[String]) -> Self {
        for p in paths {
            let expanded = expand_home(p);
            if let Ok(c) = dunce_canonicalize(&expanded) {
                self.extra_readable.push(c);
            }
        }
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether an outside path was explicitly made readable in user config.
    pub fn is_extra_readable(&self, p: &ResolvedPath) -> bool {
        !p.secret && self.extra_readable.iter().any(|e| p.abs.starts_with(e))
    }

    /// Resolve a user- or model-supplied path.
    ///
    /// Relative paths are taken relative to the workspace root. `~` expands
    /// to the home directory. The longest existing ancestor is canonicalised
    /// so symlinks that escape the workspace are detected even when the
    /// final component does not exist yet.
    pub fn resolve(&self, input: &str) -> Result<ResolvedPath, PathError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(PathError::Empty);
        }
        if input.contains('\0') {
            return Err(PathError::Nul);
        }
        let expanded = expand_home(input);
        let joined = if expanded.is_absolute() {
            expanded
        } else {
            self.root.join(expanded)
        };
        let normal = lexical_normalize(&joined);
        let abs = canonicalize_existing_prefix(&normal)
            .map_err(|e| PathError::Resolve(input.to_string(), e.to_string()))?;

        let inside = abs.starts_with(&self.root);
        let rel = if inside {
            let r = abs.strip_prefix(&self.root).unwrap_or(Path::new(""));
            let s = r
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            Some(if s.is_empty() { ".".to_string() } else { s })
        } else {
            None
        };
        let git_internal = rel
            .as_deref()
            .map(|r| r == ".git" || r.starts_with(".git/"))
            .unwrap_or(false);
        let secret = is_secret_path(&abs);
        Ok(ResolvedPath {
            abs,
            rel,
            inside,
            secret,
            git_internal,
        })
    }
}

pub fn expand_home(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(p));
    }
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(p)
}

/// Remove `.` and resolve `..` without touching the filesystem. `..` at the
/// root stays at the root.
pub fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = out.pop();
                if !popped {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn canonicalize_existing_prefix(p: &Path) -> std::io::Result<PathBuf> {
    let mut existing = p.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        // symlink_metadata so dangling symlinks still count as "existing";
        // canonicalize then fails and we fall back to reading the link.
        if existing.symlink_metadata().is_ok() {
            break;
        }
        match existing.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                if !existing.pop() {
                    break;
                }
            }
            None => break,
        }
    }
    let mut base = match dunce_canonicalize(&existing) {
        Ok(b) => b,
        Err(_) => {
            // Dangling symlink: resolve its target lexically so an escaping
            // link is still detected as outside.
            match std::fs::read_link(&existing) {
                Ok(target) => {
                    let parent = existing.parent().unwrap_or(Path::new("/"));
                    let t = if target.is_absolute() {
                        target
                    } else {
                        parent.join(target)
                    };
                    lexical_normalize(&t)
                }
                Err(_) => existing.clone(),
            }
        }
    };
    for name in tail.into_iter().rev() {
        base.push(name);
    }
    Ok(base)
}

/// `std::fs::canonicalize`, minus the `\\?\` prefix on Windows so prefix
/// comparisons with user-facing paths work.
pub fn dunce_canonicalize(p: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(p)?;
    #[cfg(windows)]
    {
        let s = c.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            if !stripped.starts_with("UNC\\") {
                return Ok(PathBuf::from(stripped));
            }
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let w = Workspace::new(dir.path()).unwrap();
        (dir, w)
    }

    #[test]
    fn relative_paths_resolve_inside() {
        let (_d, w) = ws();
        let r = w.resolve("src/main.rs").unwrap();
        assert!(r.inside);
        assert_eq!(r.rel.as_deref(), Some("src/main.rs"));
        let r = w.resolve("./src/../src/new.rs").unwrap();
        assert!(r.inside);
        assert_eq!(r.rel.as_deref(), Some("src/new.rs"));
    }

    #[test]
    fn traversal_escapes_are_detected() {
        let (_d, w) = ws();
        for p in [
            "../outside.txt",
            "src/../../outside.txt",
            "../../../../../../etc/passwd",
        ] {
            let r = w.resolve(p).unwrap();
            assert!(!r.inside, "{p} should be outside");
        }
        let r = w.resolve("/etc/passwd").unwrap();
        assert!(!r.inside);
    }

    #[test]
    fn home_paths_are_outside_and_ssh_is_secret() {
        let (_d, w) = ws();
        let r = w.resolve("~/.ssh/id_rsa").unwrap();
        assert!(!r.inside);
        assert!(r.secret);
    }

    #[test]
    fn git_internals_are_flagged() {
        let (d, w) = ws();
        std::fs::create_dir_all(d.path().join(".git/hooks")).unwrap();
        assert!(w.resolve(".git/hooks/pre-commit").unwrap().git_internal);
        assert!(!w.resolve(".github/workflows/ci.yml").unwrap().git_internal);
        assert!(!w.resolve(".gitignore").unwrap().git_internal);
    }

    #[test]
    fn rejects_nul_and_empty() {
        let (_d, w) = ws();
        assert_eq!(w.resolve(""), Err(PathError::Empty));
        assert_eq!(w.resolve("a\0b"), Err(PathError::Nul));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_detected() {
        let (d, w) = ws();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("link")).unwrap();
        let r = w.resolve("link/secret.txt").unwrap();
        assert!(!r.inside, "symlinked dir escaping the workspace");
        let r = w.resolve("link/not-yet-created.txt").unwrap();
        assert!(!r.inside, "new file under escaping symlink");

        std::os::unix::fs::symlink("/etc/passwd", d.path().join("pw")).unwrap();
        assert!(!w.resolve("pw").unwrap().inside);

        std::os::unix::fs::symlink(
            outside.path().join("dangling"),
            d.path().join("dangle"),
        )
        .unwrap();
        assert!(!w.resolve("dangle").unwrap().inside, "dangling escaping link");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_inside_workspace_is_fine() {
        let (d, w) = ws();
        std::os::unix::fs::symlink(d.path().join("src"), d.path().join("alias")).unwrap();
        let r = w.resolve("alias/main.rs").unwrap();
        assert!(r.inside);
        assert_eq!(r.rel.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn malicious_filenames_stay_literal() {
        let (d, w) = ws();
        let name = "$(rm -rf ~); `whoami` 'x'.txt";
        std::fs::write(d.path().join(name), "x").unwrap();
        let r = w.resolve(name).unwrap();
        assert!(r.inside);
        assert_eq!(r.rel.as_deref(), Some(name));
    }
}
