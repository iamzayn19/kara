//! Filesystem layout, using each platform's conventional locations.
//!
//! | | Configuration | Data (models, cache, logs, sessions, managed binaries) |
//! |---|---|---|
//! | macOS | `~/Library/Application Support/Kara/` | `~/Library/Application Support/Kara/` |
//! | Linux | `$XDG_CONFIG_HOME/kara/` (`~/.config/kara/`) | `$XDG_DATA_HOME/kara/` (`~/.local/share/kara/`) |
//! | Windows | `%APPDATA%\Kara\` | `%LOCALAPPDATA%\Kara\` |
//!
//! `KARA_HOME` overrides both with a single directory (useful for tests and
//! portable installs).
//!
//! ```text
//! <config>/config.toml       user configuration
//! <config>/models.toml       user additions to the model registry
//! <config>/languages/        user language packs
//! <data>/bin/                Kara binary managed by the editor extension
//! <data>/models/             downloaded models, shared by the CLI and the editor
//! <data>/runtimes/           managed local inference runtimes
//! <data>/cache/              repository indexes
//! <data>/logs/               runtime logs
//! <data>/sessions/           undo snapshots
//! <data>/kara.db             session history
//!
//! <project>/.kara/config.toml       optional; may only make Kara stricter
//! <project>/.kara/instructions.md   optional project conventions
//! ```

use std::path::{Path, PathBuf};

/// Directory name inside the platform's config/data roots.
pub fn app_dir_name() -> &'static str {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        "Kara"
    } else {
        "kara"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaraPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl KaraPaths {
    /// Resolve from `KARA_HOME`, else the platform directories.
    pub fn discover() -> anyhow::Result<Self> {
        if let Some(home) = std::env::var_os("KARA_HOME").filter(|v| !v.is_empty()) {
            return Ok(Self::at(PathBuf::from(home)));
        }
        let config = dirs::config_dir().ok_or_else(|| {
            anyhow::anyhow!("cannot determine the config directory; set KARA_HOME")
        })?;
        let data = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot determine the data directory; set KARA_HOME"))?;
        Ok(Self {
            config_dir: config.join(app_dir_name()),
            data_dir: data.join(app_dir_name()),
        })
    }

    /// Everything under one directory.
    pub fn at(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            config_dir: home.clone(),
            data_dir: home,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
    pub fn user_models_file(&self) -> PathBuf {
        self.config_dir.join("models.toml")
    }
    pub fn languages_dir(&self) -> PathBuf {
        self.config_dir.join("languages")
    }
    /// Secrets used to reach or serve remote inference (mode 0600 on Unix).
    pub fn credentials_dir(&self) -> PathBuf {
        self.config_dir.join("credentials")
    }
    pub fn bin_dir(&self) -> PathBuf {
        self.data_dir.join("bin")
    }
    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }
    pub fn runtimes_dir(&self) -> PathBuf {
        self.data_dir.join("runtimes")
    }
    pub fn sessions_dir(&self) -> PathBuf {
        self.data_dir.join("sessions")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }
    pub fn traces_dir(&self) -> PathBuf {
        self.data_dir.join("traces")
    }
    pub fn database(&self) -> PathBuf {
        self.data_dir.join("kara.db")
    }
    pub fn history_file(&self) -> PathBuf {
        self.data_dir.join("history")
    }

    /// Create the directory tree if missing.
    pub fn ensure(&self) -> anyhow::Result<()> {
        for dir in [
            self.config_dir.clone(),
            self.data_dir.clone(),
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

/// Write a file readable only by the current user.
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(contents)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

/// Project-local paths.
pub fn project_config_file(root: &Path) -> PathBuf {
    root.join(".kara").join("config.toml")
}

pub fn project_instructions_file(root: &Path) -> PathBuf {
    root.join(".kara").join("instructions.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kara_home_puts_everything_in_one_place() {
        let p = KaraPaths::at("/tmp/k");
        assert_eq!(p.config_file(), PathBuf::from("/tmp/k/config.toml"));
        assert_eq!(p.models_dir(), PathBuf::from("/tmp/k/models"));
        assert_eq!(p.bin_dir(), PathBuf::from("/tmp/k/bin"));
    }

    #[test]
    fn platform_directories_use_the_app_name() {
        // Do not depend on the caller's KARA_HOME.
        let config = dirs::config_dir().unwrap().join(app_dir_name());
        let data = dirs::data_local_dir().unwrap().join(app_dir_name());
        assert!(config.ends_with(app_dir_name()));
        assert!(data.ends_with(app_dir_name()));
        if cfg!(target_os = "macos") {
            assert!(data
                .to_string_lossy()
                .contains("Library/Application Support/Kara"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("c/token");
        write_private(&f, b"secret").unwrap();
        let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
