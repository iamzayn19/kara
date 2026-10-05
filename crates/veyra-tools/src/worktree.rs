//! Detect files changed by shell commands so they can be journaled.
//!
//! File tools journal their own writes. Shell commands (formatters, code
//! generators, `sed -i`) can also change files, so before such a command
//! Veyra records size+mtime of every non-ignored file and compares afterwards.
//! For changed files whose prior content is recoverable (clean tracked files
//! via `git show HEAD:path`, files Veyra already snapshotted, newly created
//! files) the change is added to the undo journal. Anything else is reported
//! as not undoable.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::time::UNIX_EPOCH;
use veyra_context::git;
use veyra_context::search::{rel_path, walk_files};

use crate::journal::Journal;

#[derive(Debug, Default)]
pub struct WorktreeSnapshot {
    files: HashMap<String, (u64, i64)>,
    dirty: BTreeSet<String>,
    is_git: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExternalChanges {
    pub journaled: Vec<String>,
    pub not_undoable: Vec<String>,
}

impl WorktreeSnapshot {
    pub fn capture(root: &Path) -> WorktreeSnapshot {
        let mut files = HashMap::new();
        for p in walk_files(root, None) {
            if let Ok(md) = std::fs::metadata(&p) {
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0);
                files.insert(rel_path(root, &p), (md.len(), mtime));
            }
        }
        let is_git = git::is_repo(root);
        let dirty = if is_git {
            git::dirty_paths(root).into_iter().collect()
        } else {
            BTreeSet::new()
        };
        WorktreeSnapshot {
            files,
            dirty,
            is_git,
        }
    }

    /// Compare with the current state and journal what can be undone.
    pub fn journal_changes(&self, root: &Path, journal: &mut Journal) -> ExternalChanges {
        let after = WorktreeSnapshot::capture(root);
        let mut changed: BTreeSet<String> = BTreeSet::new();
        for (p, meta) in &after.files {
            if self.files.get(p) != Some(meta) {
                changed.insert(p.clone());
            }
        }
        for p in self.files.keys() {
            if !after.files.contains_key(p) {
                changed.insert(p.clone());
            }
        }

        let mut out = ExternalChanges::default();
        for rel in changed {
            if journal.tracks_in_open_batch(&rel) {
                // Snapshot already exists from an earlier tool write.
                let now = std::fs::read(root.join(&rel)).ok();
                journal.after_mutation(&rel, now.as_deref());
                out.journaled.push(rel);
                continue;
            }
            let existed_before = self.files.contains_key(&rel);
            let before: Option<Option<Vec<u8>>> = if !existed_before {
                Some(None)
            } else if self.is_git && !self.dirty.contains(&rel) {
                git_show_head(root, &rel).map(Some)
            } else {
                None
            };
            match before {
                Some(before) => {
                    let now = std::fs::read(root.join(&rel)).ok();
                    if journal
                        .record_external(&rel, before.as_deref(), now.as_deref())
                        .is_ok()
                    {
                        out.journaled.push(rel);
                    } else {
                        out.not_undoable.push(rel);
                    }
                }
                None => out.not_undoable.push(rel),
            }
        }
        out
    }
}

fn git_show_head(root: &Path, rel: &str) -> Option<Vec<u8>> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &format!("HEAD:{rel}")])
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}
