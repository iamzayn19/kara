//! Model registry: data describing downloadable models.
//!
//! The built-in registry is `models/registry.toml`, compiled into the binary.
//! Users may add or override entries in `~/.veyra/models.toml`.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const BUILTIN_REGISTRY: &str = include_str!("../../../models/registry.toml");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub family: String,
    pub quantization: String,
    pub params_b: f64,
    pub active_params_b: f64,
    pub architecture: String,
    pub license: String,
    pub source: String,
    pub repo: String,
    pub file: String,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub native_context: u32,
    pub default_context: u32,
    pub min_context: u32,
    pub kv_bytes_per_token: u64,
    pub tool_calling: String,
    pub tier: String,
    pub quality_rank: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_score: Option<f64>,
    #[serde(default = "yes")]
    pub auto_select: bool,
    /// Default thinking-token budget per response (-1 = unlimited). Small
    /// models on modest hardware spend most of their time thinking; a budget
    /// keeps agent steps responsive. Users override with model.reasoning_budget.
    #[serde(default = "unlimited")]
    pub reasoning_budget: i32,
    #[serde(default)]
    pub notes: String,
}

fn yes() -> bool {
    true
}

fn unlimited() -> i32 {
    -1
}

#[derive(Debug, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    model: Vec<ModelSpec>,
}

impl ModelSpec {
    pub fn download_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.file
        )
    }

    pub fn source_url(&self) -> String {
        format!("https://huggingface.co/{}", self.repo)
    }

    pub fn size_gb(&self) -> f64 {
        self.size_bytes as f64 / 1e9
    }

    /// Estimated memory needed to run with `ctx` tokens of context.
    pub fn memory_needed(&self, ctx: u32) -> u64 {
        const OVERHEAD: u64 = 700 * 1024 * 1024;
        self.size_bytes + self.kv_bytes_per_token * ctx as u64 + OVERHEAD
    }

    pub fn is_moe(&self) -> bool {
        self.architecture == "moe"
    }

    /// Score used for automatic selection: measured evaluation results win
    /// over the provisional rank once they exist.
    pub fn selection_score(&self) -> f64 {
        match self.eval_score {
            Some(s) => 1000.0 + s * 1000.0,
            None => self.quality_rank as f64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Registry {
    pub models: Vec<ModelSpec>,
}

impl Registry {
    pub fn parse(text: &str) -> anyhow::Result<Registry> {
        let f: RegistryFile = toml::from_str(text)?;
        Ok(Registry { models: f.model })
    }

    pub fn builtin() -> Registry {
        Self::parse(BUILTIN_REGISTRY).expect("built-in model registry is valid")
    }

    /// Built-in registry overlaid with the user's registry file, if any.
    pub fn load(user_file: &Path) -> anyhow::Result<Registry> {
        let mut reg = Self::builtin();
        if user_file.exists() {
            let user = Self::parse(&std::fs::read_to_string(user_file)?)
                .map_err(|e| anyhow::anyhow!("invalid {}: {e}", user_file.display()))?;
            for m in user.models {
                reg.models.retain(|x| x.id != m.id);
                reg.models.push(m);
            }
        }
        Ok(reg)
    }

    pub fn get(&self, id: &str) -> Option<&ModelSpec> {
        let id = id.to_ascii_lowercase();
        self.models
            .iter()
            .find(|m| m.id == id || m.name.to_ascii_lowercase() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_is_complete_and_pinned() {
        let r = Registry::builtin();
        assert!(r.models.len() >= 5);
        for m in &r.models {
            assert_eq!(
                m.revision.len(),
                40,
                "{} revision must be a commit sha",
                m.id
            );
            let sha = m.sha256.as_deref().expect("sha256 recorded");
            assert_eq!(sha.len(), 64, "{}", m.id);
            assert!(m.size_bytes > 0);
            assert!(m.default_context >= m.min_context);
            assert!(m.default_context <= m.native_context);
            assert!(!m.license.is_empty());
            assert!(m.download_url().starts_with("https://huggingface.co/"));
            assert!(m.download_url().contains(&m.revision));
        }
        assert!(r.get("qwen3.6-35b-a3b-q4_k_m").unwrap().is_moe());
        assert!(r.get("Qwen3.6-27B").is_some());
    }

    #[test]
    fn user_registry_overrides() {
        let dir = std::env::temp_dir().join(format!("veyra-reg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("models.toml");
        let mut m = Registry::builtin().models[0].clone();
        m.quality_rank = 1;
        let text = toml::to_string(&RegistryFile {
            model: vec![m.clone()],
        })
        .unwrap();
        std::fs::write(&f, text).unwrap();
        let r = Registry::load(&f).unwrap();
        assert_eq!(r.get(&m.id).unwrap().quality_rank, 1);
        assert_eq!(r.models.len(), Registry::builtin().models.len());
        std::fs::remove_dir_all(dir).ok();
    }
}
