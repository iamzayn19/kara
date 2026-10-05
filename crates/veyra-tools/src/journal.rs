//! Mutation journal for `/undo`.
//!
//! Every Veyra file mutation belongs to a *batch* (normally one user turn).
//! Before the first change to a path in a batch, the journal stores the exact
//! prior bytes (or records that the file did not exist). After each change it
//! stores a hash of what Veyra wrote.
//!
//! Undo restores the prior bytes, but only for files that still contain what
//! Veyra wrote. If the user edited a file afterwards, that file is reported as
//! a conflict and left untouched, so undo never destroys user work. Because
//! the snapshot is the exact pre-Veyra content, pre-existing uncommitted user
//! changes are preserved too: no git reset, checkout or stash is involved.
//!
//! Snapshots are persisted under the session directory so undo survives a
//! restart.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub fn hash_bytes(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// Workspace-relative path.
    pub path: String,
    /// Hash of the content before Veyra touched it in this batch; `None` if
    /// the file did not exist.
    pub before_hash: Option<String>,
    /// Hash after Veyra's latest write in this batch; `None` if Veyra deleted it.
    pub after_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Batch {
    pub id: u64,
    pub label: String,
    pub created: String,
    pub git_head: Option<String>,
    pub entries: Vec<Entry>,
    pub undone: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UndoReport {
    pub batch_id: u64,
    pub restored: Vec<String>,
    /// Files changed by someone else after Veyra's edit; left untouched.
    pub conflicts: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct Persisted {
    batches: Vec<Batch>,
    next_id: u64,
    baseline_dirty: BTreeSet<String>,
}

#[derive(Debug)]
pub struct Journal {
    root: PathBuf,
    /// Where snapshots and the journal index are stored.
    dir: PathBuf,
    batches: Vec<Batch>,
    open: Option<usize>,
    next_id: u64,
    /// Paths with uncommitted user changes when the session started.
    baseline_dirty: BTreeSet<String>,
    /// Content hash at the time Veyra last read or wrote a path, to detect
    /// concurrent user edits before overwriting.
    seen: BTreeMap<String, String>,
}

impl Journal {
    /// Open or create a journal stored in `dir` for the workspace `root`.
    pub fn open(root: &Path, dir: &Path) -> anyhow::Result<Journal> {
        std::fs::create_dir_all(dir.join("snapshots"))?;
        let index = dir.join("journal.json");
        let persisted: Persisted = if index.exists() {
            serde_json::from_slice(&std::fs::read(&index)?).unwrap_or_default()
        } else {
            Persisted::default()
        };
        Ok(Journal {
            root: root.to_path_buf(),
            dir: dir.to_path_buf(),
            batches: persisted.batches,
            open: None,
            next_id: persisted.next_id.max(1),
            baseline_dirty: persisted.baseline_dirty,
            seen: BTreeMap::new(),
        })
    }

    pub fn set_baseline_dirty(&mut self, paths: impl IntoIterator<Item = String>) {
        self.baseline_dirty = paths.into_iter().collect();
        let _ = self.save();
    }

    pub fn baseline_dirty(&self) -> &BTreeSet<String> {
        &self.baseline_dirty
    }

    /// Start a new batch (closing any open one).
    pub fn begin(&mut self, label: impl Into<String>, git_head: Option<String>) -> u64 {
        self.end();
        let id = self.next_id;
        self.next_id += 1;
        self.batches.push(Batch {
            id,
            label: label.into(),
            created: chrono::Utc::now().to_rfc3339(),
            git_head,
            entries: Vec::new(),
            undone: false,
        });
        self.open = Some(self.batches.len() - 1);
        id
    }

    /// Close the open batch. Empty batches are dropped.
    pub fn end(&mut self) {
        if let Some(i) = self.open.take() {
            if self.batches[i].entries.is_empty() {
                self.batches.remove(i);
            }
            let _ = self.save();
        }
    }

    pub fn current_batch(&self) -> Option<u64> {
        self.open.map(|i| self.batches[i].id)
    }

    pub fn batches(&self) -> &[Batch] {
        &self.batches
    }

    /// Record that Veyra observed `content` at `rel`.
    pub fn note_seen(&mut self, rel: &str, content: &[u8]) {
        self.seen.insert(rel.to_string(), hash_bytes(content));
    }

    /// Check whether `rel` still matches what Veyra last saw. Returns
    /// `Ok(true)` if unchanged, `Ok(false)` if never seen.
    pub fn verify_unchanged(&self, rel: &str) -> Result<bool, String> {
        let Some(expected) = self.seen.get(rel) else {
            return Ok(false);
        };
        let current = std::fs::read(self.root.join(rel)).ok().map(|b| hash_bytes(&b));
        match current {
            Some(h) if &h == expected => Ok(true),
            Some(_) => Err(format!(
                "{rel} changed on disk since Veyra last read it (probably edited by the user); read it again before editing"
            )),
            None => Err(format!("{rel} was deleted since Veyra last read it")),
        }
    }

    /// Call before mutating `rel`. Snapshots prior content on first touch in
    /// the open batch. Opens an implicit batch if none is open.
    pub fn before_mutation(&mut self, rel: &str) -> anyhow::Result<()> {
        if self.open.is_none() {
            self.begin("edit", None);
        }
        let bi = self.open.expect("open batch");
        if self.batches[bi].entries.iter().any(|e| e.path == rel) {
            return Ok(());
        }
        let abs = self.root.join(rel);
        let before_hash = match std::fs::read(&abs) {
            Ok(bytes) => {
                let h = hash_bytes(&bytes);
                let snap = self.snapshot_path(&h);
                if !snap.exists() {
                    std::fs::write(&snap, &bytes)?;
                }
                Some(h)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        self.batches[bi].entries.push(Entry {
            path: rel.to_string(),
            before_hash: before_hash.clone(),
            after_hash: before_hash,
        });
        self.save()?;
        Ok(())
    }

    /// Call after mutating `rel` with the new content (`None` = deleted).
    pub fn after_mutation(&mut self, rel: &str, content: Option<&[u8]>) {
        let h = content.map(hash_bytes);
        if let Some(bi) = self.open {
            if let Some(e) = self.batches[bi].entries.iter_mut().find(|e| e.path == rel) {
                e.after_hash = h.clone();
            }
        }
        match h {
            Some(h) => {
                self.seen.insert(rel.to_string(), h);
            }
            None => {
                self.seen.remove(rel);
            }
        }
        let _ = self.save();
    }

    /// Record a change made outside the file tools (by a shell command) when
    /// the prior content is known, e.g. from `git show HEAD:path` for a file
    /// that was clean before the command ran.
    pub fn record_external(
        &mut self,
        rel: &str,
        before: Option<&[u8]>,
        after: Option<&[u8]>,
    ) -> anyhow::Result<()> {
        if self.open.is_none() {
            self.begin("shell", None);
        }
        let bi = self.open.expect("open batch");
        if !self.batches[bi].entries.iter().any(|e| e.path == rel) {
            let before_hash = match before {
                Some(bytes) => {
                    let h = hash_bytes(bytes);
                    let snap = self.snapshot_path(&h);
                    if !snap.exists() {
                        std::fs::write(&snap, bytes)?;
                    }
                    Some(h)
                }
                None => None,
            };
            self.batches[bi].entries.push(Entry {
                path: rel.to_string(),
                before_hash,
                after_hash: None,
            });
        }
        self.after_mutation(rel, after);
        Ok(())
    }

    /// Whether the open batch already tracks `rel`.
    pub fn tracks_in_open_batch(&self, rel: &str) -> bool {
        self.open
            .map(|i| self.batches[i].entries.iter().any(|e| e.path == rel))
            .unwrap_or(false)
    }

    /// Prior content for `rel` in the given batch (for diffs and previews).
    pub fn before_content(&self, batch: u64, rel: &str) -> Option<Option<Vec<u8>>> {
        let b = self.batches.iter().find(|b| b.id == batch)?;
        let e = b.entries.iter().find(|e| e.path == rel)?;
        Some(e.before_hash.as_ref().and_then(|h| std::fs::read(self.snapshot_path(h)).ok()))
    }

    /// Earliest pre-Veyra content of `rel` across all live batches.
    pub fn original_content(&self, rel: &str) -> Option<Option<Vec<u8>>> {
        let b = self
            .batches
            .iter()
            .filter(|b| !b.undone)
            .find(|b| b.entries.iter().any(|e| e.path == rel))?;
        self.before_content(b.id, rel)
    }

    /// Paths Veyra changed in batches that have not been undone.
    pub fn veyra_changed_paths(&self) -> BTreeSet<String> {
        self.batches
            .iter()
            .filter(|b| !b.undone)
            .flat_map(|b| b.entries.iter())
            .filter(|e| e.before_hash != e.after_hash)
            .map(|e| e.path.clone())
            .collect()
    }

    pub fn last_live_batch(&self) -> Option<&Batch> {
        self.batches.iter().rev().find(|b| !b.undone && !b.entries.is_empty())
    }

    /// Undo the most recent live batch (or a specific one).
    pub fn undo(&mut self, batch: Option<u64>) -> anyhow::Result<UndoReport> {
        self.end();
        let idx = match batch {
            Some(id) => self.batches.iter().position(|b| b.id == id && !b.undone),
            None => self
                .batches
                .iter()
                .rposition(|b| !b.undone && !b.entries.is_empty()),
        }
        .ok_or_else(|| anyhow::anyhow!("nothing to undo"))?;

        let mut report = UndoReport {
            batch_id: self.batches[idx].id,
            ..Default::default()
        };
        let entries = self.batches[idx].entries.clone();
        for e in entries.iter().rev() {
            let abs = self.root.join(&e.path);
            let current = std::fs::read(&abs).ok().map(|b| hash_bytes(&b));
            if current == e.before_hash {
                // Already in the original state.
                continue;
            }
            if current != e.after_hash {
                report.conflicts.push(e.path.clone());
                continue;
            }
            let res = match &e.before_hash {
                Some(h) => std::fs::read(self.snapshot_path(h))
                    .and_then(|bytes| {
                        if let Some(parent) = abs.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(&abs, bytes)
                    }),
                None => std::fs::remove_file(&abs),
            };
            match res {
                Ok(()) => {
                    report.restored.push(e.path.clone());
                    self.seen.remove(&e.path);
                }
                Err(err) => report.errors.push(format!("{}: {err}", e.path)),
            }
        }
        // A batch with conflicts stays partially live so the user can resolve.
        if report.conflicts.is_empty() && report.errors.is_empty() {
            self.batches[idx].undone = true;
        } else {
            let restored: BTreeSet<&String> = report.restored.iter().collect();
            self.batches[idx]
                .entries
                .retain(|e| !restored.contains(&e.path));
        }
        self.save()?;
        Ok(report)
    }

    fn snapshot_path(&self, hash: &str) -> PathBuf {
        self.dir.join("snapshots").join(hash)
    }

    fn save(&self) -> anyhow::Result<()> {
        let p = Persisted {
            batches: self.batches.clone(),
            next_id: self.next_id,
            baseline_dirty: self.baseline_dirty.clone(),
        };
        let tmp = self.dir.join("journal.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&p)?)?;
        std::fs::rename(tmp, self.dir.join("journal.json"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, Journal) {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let j = Journal::open(root.path(), state.path()).unwrap();
        (root, state, j)
    }

    fn write(root: &Path, rel: &str, s: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    fn read(root: &Path, rel: &str) -> Option<String> {
        std::fs::read_to_string(root.join(rel)).ok()
    }

    fn veyra_write(j: &mut Journal, root: &Path, rel: &str, s: &str) {
        j.before_mutation(rel).unwrap();
        write(root, rel, s);
        j.after_mutation(rel, Some(s.as_bytes()));
    }

    #[test]
    fn undo_restores_edits_and_removes_created_files() {
        let (root, _s, mut j) = setup();
        let r = root.path();
        write(r, "a.txt", "original");
        j.begin("turn 1", None);
        veyra_write(&mut j, r, "a.txt", "changed once");
        veyra_write(&mut j, r, "a.txt", "changed twice");
        veyra_write(&mut j, r, "new/b.txt", "created");
        j.end();
        assert_eq!(j.veyra_changed_paths().len(), 2);

        let rep = j.undo(None).unwrap();
        assert_eq!(read(r, "a.txt").as_deref(), Some("original"));
        assert_eq!(read(r, "new/b.txt"), None);
        assert_eq!(rep.restored.len(), 2);
        assert!(rep.conflicts.is_empty());
        assert!(j.veyra_changed_paths().is_empty());
        assert!(j.undo(None).is_err(), "nothing left to undo");
    }

    #[test]
    fn undo_preserves_preexisting_uncommitted_user_changes() {
        let (root, _s, mut j) = setup();
        let r = root.path();
        // User has uncommitted work before Veyra starts.
        write(r, "app.rb", "committed\n+ user wip\n");
        j.set_baseline_dirty(["app.rb".to_string()]);
        j.begin("turn", None);
        veyra_write(&mut j, r, "app.rb", "committed\n+ user wip\n+ veyra fix\n");
        j.end();
        j.undo(None).unwrap();
        assert_eq!(read(r, "app.rb").as_deref(), Some("committed\n+ user wip\n"));
    }

    #[test]
    fn undo_never_overwrites_later_user_edits() {
        let (root, _s, mut j) = setup();
        let r = root.path();
        write(r, "a.txt", "v0");
        write(r, "b.txt", "b0");
        j.begin("turn", None);
        veyra_write(&mut j, r, "a.txt", "v1");
        veyra_write(&mut j, r, "b.txt", "b1");
        j.end();
        write(r, "a.txt", "user edited after veyra");
        let rep = j.undo(None).unwrap();
        assert_eq!(rep.conflicts, vec!["a.txt"]);
        assert_eq!(rep.restored, vec!["b.txt"]);
        assert_eq!(read(r, "a.txt").as_deref(), Some("user edited after veyra"));
        assert_eq!(read(r, "b.txt").as_deref(), Some("b0"));
    }

    #[test]
    fn undo_is_per_batch_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let r = root.path();
        write(r, "a.txt", "v0");
        {
            let mut j = Journal::open(r, state.path()).unwrap();
            j.begin("turn 1", None);
            veyra_write(&mut j, r, "a.txt", "v1");
            j.begin("turn 2", None);
            veyra_write(&mut j, r, "a.txt", "v2");
            j.end();
        }
        let mut j = Journal::open(r, state.path()).unwrap();
        j.undo(None).unwrap();
        assert_eq!(read(r, "a.txt").as_deref(), Some("v1"));
        j.undo(None).unwrap();
        assert_eq!(read(r, "a.txt").as_deref(), Some("v0"));
    }

    #[test]
    fn deleted_files_are_restored() {
        let (root, _s, mut j) = setup();
        let r = root.path();
        write(r, "gone.txt", "keep me");
        j.begin("turn", None);
        j.before_mutation("gone.txt").unwrap();
        std::fs::remove_file(r.join("gone.txt")).unwrap();
        j.after_mutation("gone.txt", None);
        j.end();
        j.undo(None).unwrap();
        assert_eq!(read(r, "gone.txt").as_deref(), Some("keep me"));
    }

    #[test]
    fn stale_write_detection() {
        let (root, _s, mut j) = setup();
        let r = root.path();
        write(r, "a.txt", "v0");
        assert_eq!(j.verify_unchanged("a.txt"), Ok(false));
        j.note_seen("a.txt", b"v0");
        assert_eq!(j.verify_unchanged("a.txt"), Ok(true));
        write(r, "a.txt", "user");
        assert!(j.verify_unchanged("a.txt").is_err());
    }
}
