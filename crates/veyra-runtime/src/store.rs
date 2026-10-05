//! Local model store.
//!
//! ```text
//! ~/.veyra/models/<owner>__<repo>/<revision[:12]>/<file>.gguf
//! ~/.veyra/models/<owner>__<repo>/<revision[:12]>/veyra-model.json
//! ```
//!
//! The JSON record keeps provenance: source repository, pinned revision,
//! license, SHA-256 and download time.

use crate::download::{download_verified, sha256_file, Progress};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;
use veyra_model::registry::ModelSpec;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRecord {
    pub id: String,
    pub name: String,
    pub source: String,
    pub repo: String,
    pub revision: String,
    pub file: String,
    pub license: String,
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub downloaded_at: String,
}

pub struct ModelStore {
    pub dir: PathBuf,
}

impl ModelStore {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf() }
    }

    fn model_dir(&self, spec: &ModelSpec) -> PathBuf {
        let rev: String = spec.revision.chars().take(12).collect();
        self.dir.join(spec.repo.replace('/', "__")).join(rev)
    }

    pub fn path_for(&self, spec: &ModelSpec) -> PathBuf {
        self.model_dir(spec).join(&spec.file)
    }

    pub fn record(&self, spec: &ModelSpec) -> Option<ModelRecord> {
        let p = self.model_dir(spec).join("veyra-model.json");
        serde_json::from_slice(&std::fs::read(p).ok()?).ok()
    }

    /// Installed means: file present with the expected size and a record
    /// written after successful checksum verification.
    pub fn is_installed(&self, spec: &ModelSpec) -> bool {
        let p = self.path_for(spec);
        let size_ok = std::fs::metadata(&p).map(|m| m.len() == spec.size_bytes).unwrap_or(false);
        size_ok && self.record(spec).is_some()
    }

    /// Bytes already downloaded for an interrupted download.
    pub fn partial_bytes(&self, spec: &ModelSpec) -> u64 {
        std::fs::metadata(crate::download::part_path(&self.path_for(spec)))
            .map(|m| m.len())
            .unwrap_or(0)
    }

    pub async fn download(
        &self,
        spec: &ModelSpec,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
        cancel: &CancellationToken,
    ) -> anyhow::Result<PathBuf> {
        let dest = self.path_for(spec);
        let client = reqwest::Client::builder().build()?;
        download_verified(&client, &spec.download_url(), &dest, spec.sha256.as_deref(), on_progress, cancel)
            .await
            .map_err(|e| anyhow::anyhow!("{}: {e}", spec.name))?;
        let rec = ModelRecord {
            id: spec.id.clone(),
            name: spec.name.clone(),
            source: spec.source_url(),
            repo: spec.repo.clone(),
            revision: spec.revision.clone(),
            file: spec.file.clone(),
            license: spec.license.clone(),
            sha256: spec.sha256.clone(),
            size_bytes: spec.size_bytes,
            downloaded_at: chrono::Utc::now().to_rfc3339(),
        };
        std::fs::write(
            self.model_dir(spec).join("veyra-model.json"),
            serde_json::to_vec_pretty(&rec)?,
        )?;
        Ok(dest)
    }

    /// Re-hash an installed model (for `veyra models verify`).
    pub fn verify(&self, spec: &ModelSpec) -> anyhow::Result<bool> {
        let Some(expected) = &spec.sha256 else {
            anyhow::bail!("no checksum recorded for {}", spec.id)
        };
        Ok(sha256_file(&self.path_for(spec))?.eq_ignore_ascii_case(expected))
    }

    /// Installed model records.
    pub fn installed(&self) -> Vec<ModelRecord> {
        let mut out = Vec::new();
        let Ok(repos) = std::fs::read_dir(&self.dir) else { return out };
        for repo in repos.flatten() {
            let Ok(revs) = std::fs::read_dir(repo.path()) else { continue };
            for rev in revs.flatten() {
                if let Ok(bytes) = std::fs::read(rev.path().join("veyra-model.json")) {
                    if let Ok(r) = serde_json::from_slice::<ModelRecord>(&bytes) {
                        out.push(r);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veyra_model::registry::Registry;

    #[test]
    fn layout_is_pinned_to_revision() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModelStore::new(dir.path());
        let spec = Registry::builtin().get("qwen3-4b-q4_k_m").unwrap().clone();
        let p = store.path_for(&spec);
        assert!(p.to_string_lossy().contains("ggml-org__Qwen3-4B-GGUF"));
        assert!(p.to_string_lossy().contains(&spec.revision[..12]));
        assert!(!store.is_installed(&spec));
        // Correct size alone is not enough without a verified record.
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(spec.size_bytes).unwrap();
        assert!(!store.is_installed(&spec));
        assert!(store.installed().is_empty());
    }
}
