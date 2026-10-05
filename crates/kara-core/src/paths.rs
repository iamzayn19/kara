//! Filesystem layout.
//!
//! ```text
//! ~/.kara/                 (override with KARA_HOME)
//!   config.toml
//!   models/                 downloaded GGUF files
//!   runtimes/               managed llama.cpp builds
//!   sessions/               per-session logs and traces
//!   cache/                  repository indexes
//!   logs/                   runtime logs
//!   kara.db                sessions, task state, undo journal
//!
//! <project>/.kara/         optional, user-authored
//!   config.toml
//!   instructions.md
//! ```

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct KaraPaths {
    pub home: PathBuf,
}

impl KaraPaths {
    /// Resolve the Kara home directory from `KARA_HOME` or `~/.kara`.
    pub fn discover() -> anyhow::Result<Self> {
        if let Some(home) = std::env::var_os("KARA_HOME").filter(|v| !v.is_empty()) {
            return Ok(Self {
                home: PathBuf::from(home),
            });
        }
        let base = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot determine home directory; set KARA_HOME"))?;
        Ok(Self {
            home: base.join(".kara"),
        })
    }

    pub fn at(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn config_file(&self) -> PathBuf {
        self.home.join("config.toml")
    }
    pub fn models_dir(&self) -> PathBuf {
        self.home.join("models")
    }
    pub fn runtimes_dir(&self) -> PathBuf {
        self.home.join("runtimes")
    }
    pub fn sessions_dir(&self) -> PathBuf {
        self.home.join("sessions")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.home.join("cache")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.home.join("logs")
    }
    pub fn traces_dir(&self) -> PathBuf {
        self.home.join("traces")
    }
    pub fn database(&self) -> PathBuf {
        self.home.join("kara.db")
    }
    pub fn history_file(&self) -> PathBuf {
        self.home.join("history")
    }
    pub fn user_models_file(&self) -> PathBuf {
        self.home.join("models.toml")
    }

    /// Create the directory tree if missing.
    pub fn ensure(&self) -> anyhow::Result<()> {
        for dir in [
            self.home.clone(),
            self.models_dir(),
            self.runtimes_dir(),
            self.sessions_dir(),
            self.cache_dir(),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }
}

/// Project-local paths.
pub fn project_config_file(root: &Path) -> PathBuf {
    root.join(".kara").join("config.toml")
}

pub fn project_instructions_file(root: &Path) -> PathBuf {
    root.join(".kara").join("instructions.md")
}
