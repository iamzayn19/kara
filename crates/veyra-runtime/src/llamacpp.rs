//! Locate or install llama.cpp's `llama-server`.
//!
//! Search order:
//! 1. `runtime.llama_server_path` from config
//! 2. `llama-server` on PATH
//! 3. the Veyra-managed install under `~/.veyra/runtimes/llama.cpp/<tag>/`
//!
//! Installs use the pinned release in `models/runtimes.toml`, chosen for the
//! detected OS, CPU architecture and accelerator, verified by SHA-256.

use crate::archive;
use crate::download::{download_verified, Progress};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;
use veyra_model::hardware::HardwareInfo;

pub const RUNTIME_MANIFEST: &str = include_str!("../../../models/runtimes.toml");

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Asset {
    pub os: String,
    pub arch: String,
    pub accel: String,
    pub name: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub extra_name: Option<String>,
    #[serde(default)]
    pub extra_sha256: Option<String>,
    #[serde(default)]
    pub extra_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlamaCppPin {
    pub tag: String,
    pub repo: String,
    pub license: String,
    #[serde(rename = "asset")]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    llama_cpp: LlamaCppPin,
}

impl LlamaCppPin {
    pub fn builtin() -> LlamaCppPin {
        let m: ManifestFile = toml::from_str(RUNTIME_MANIFEST).expect("runtime manifest is valid");
        m.llama_cpp
    }

    pub fn url(&self, name: &str) -> String {
        format!("https://github.com/{}/releases/download/{}/{}", self.repo, self.tag, name)
    }

    /// Pick the best asset for this machine. GPU builds are preferred when
    /// the matching driver stack was detected; CPU builds are the fallback.
    pub fn select(&self, hw: &HardwareInfo) -> Option<&Asset> {
        let order: Vec<&str> = match hw.os.as_str() {
            "macos" => vec!["metal", "cpu"],
            _ => {
                let mut v = Vec::new();
                if hw.cuda {
                    v.push("cuda");
                }
                if hw.rocm {
                    v.push("rocm");
                }
                if hw.vulkan {
                    v.push("vulkan");
                }
                v.push("cpu");
                v
            }
        };
        order.iter().find_map(|accel| {
            self.assets
                .iter()
                .find(|a| a.os == hw.os && a.arch == hw.arch && a.accel == *accel)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallRecord {
    pub tag: String,
    pub asset: String,
    pub sha256: String,
    pub binary: PathBuf,
    pub installed_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Located {
    Configured(PathBuf),
    Path(PathBuf),
    Managed(InstallRecord),
}

impl Located {
    pub fn binary(&self) -> &Path {
        match self {
            Located::Configured(p) | Located::Path(p) => p,
            Located::Managed(r) => &r.binary,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Located::Configured(p) => format!("{} (from config)", p.display()),
            Located::Path(p) => format!("{} (on PATH)", p.display()),
            Located::Managed(r) => format!("{} (managed, llama.cpp {})", r.binary.display(), r.tag),
        }
    }
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

pub struct LlamaCppManager {
    pub runtimes_dir: PathBuf,
    pub pin: LlamaCppPin,
}

impl LlamaCppManager {
    pub fn new(runtimes_dir: &Path) -> Self {
        Self {
            runtimes_dir: runtimes_dir.join("llama.cpp"),
            pin: LlamaCppPin::builtin(),
        }
    }

    fn record_path(&self) -> PathBuf {
        self.runtimes_dir.join(&self.pin.tag).join("veyra-install.json")
    }

    pub fn locate(&self, configured: &str) -> Option<Located> {
        if !configured.trim().is_empty() {
            let p = PathBuf::from(configured.trim());
            return p.is_file().then_some(Located::Configured(p));
        }
        if let Some(p) = which(exe_name()) {
            return Some(Located::Path(p));
        }
        let rec: InstallRecord = serde_json::from_slice(&std::fs::read(self.record_path()).ok()?).ok()?;
        rec.binary.is_file().then_some(Located::Managed(rec))
    }

    /// Download, verify and extract the pinned build for this machine.
    pub async fn install(
        &self,
        hw: &HardwareInfo,
        on_progress: &(dyn Fn(&str, Progress) + Send + Sync),
        cancel: &CancellationToken,
    ) -> anyhow::Result<InstallRecord> {
        let asset = self.pin.select(hw).ok_or_else(|| {
            anyhow::anyhow!(
                "no prebuilt llama.cpp for {}/{}; install llama-server yourself and set runtime.llama_server_path",
                hw.os,
                hw.arch
            )
        })?;
        let client = reqwest::Client::builder().build()?;
        let dir = self.runtimes_dir.join(&self.pin.tag);
        let dl = self.runtimes_dir.join("downloads");
        std::fs::create_dir_all(&dl)?;

        let mut files = vec![(asset.name.clone(), asset.sha256.clone())];
        if let (Some(n), Some(s)) = (&asset.extra_name, &asset.extra_sha256) {
            files.push((n.clone(), s.clone()));
        }
        for (name, sha) in &files {
            let dest = dl.join(name);
            let label = name.clone();
            download_verified(&client, &self.pin.url(name), &dest, Some(sha), &|p| on_progress(&label, p), cancel)
                .await
                .map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
            archive::extract(&dest, &dir)?;
            let _ = std::fs::remove_file(&dest);
        }

        let binary = find_file(&dir, exe_name())
            .ok_or_else(|| anyhow::anyhow!("{} not found in {}", exe_name(), asset.name))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = std::fs::metadata(&binary)?.permissions();
            perm.set_mode(perm.mode() | 0o755);
            std::fs::set_permissions(&binary, perm)?;
        }
        #[cfg(target_os = "macos")]
        {
            // Downloads from the internet may carry a quarantine attribute.
            let _ = std::process::Command::new("xattr")
                .args(["-dr", "com.apple.quarantine"])
                .arg(&dir)
                .status();
        }
        let rec = InstallRecord {
            tag: self.pin.tag.clone(),
            asset: asset.name.clone(),
            sha256: asset.sha256.clone(),
            binary,
            installed_at: chrono::Utc::now().to_rfc3339(),
        };
        std::fs::write(self.record_path(), serde_json::to_vec_pretty(&rec)?)?;
        Ok(rec)
    }
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().map(|n| n == name).unwrap_or(false) {
                return Some(p);
            }
        }
    }
    None
}

/// `llama-server --version`, first useful line.
pub fn version(binary: &Path) -> Option<String> {
    let out = std::process::Command::new(binary).arg("--version").output().ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    text.lines().find(|l| l.starts_with("version")).map(|l| l.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_complete() {
        let pin = LlamaCppPin::builtin();
        assert!(pin.tag.starts_with('b'));
        for a in &pin.assets {
            assert_eq!(a.sha256.len(), 64, "{}", a.name);
            assert!(a.name.contains(&pin.tag) || a.name.starts_with("cudart"), "{}", a.name);
        }
    }

    #[test]
    fn asset_selection() {
        let pin = LlamaCppPin::builtin();
        let mac = HardwareInfo { os: "macos".into(), arch: "aarch64".into(), ..Default::default() };
        assert!(pin.select(&mac).unwrap().name.contains("macos-arm64"));
        let linux_cuda = HardwareInfo { os: "linux".into(), arch: "x86_64".into(), cuda: true, vulkan: true, ..Default::default() };
        assert!(pin.select(&linux_cuda).unwrap().name.contains("cuda"));
        let linux_vk = HardwareInfo { os: "linux".into(), arch: "x86_64".into(), vulkan: true, ..Default::default() };
        assert!(pin.select(&linux_vk).unwrap().name.contains("vulkan"));
        let linux_arm = HardwareInfo { os: "linux".into(), arch: "aarch64".into(), cuda: true, ..Default::default() };
        assert!(pin.select(&linux_arm).unwrap().name.contains("ubuntu-arm64"), "CPU fallback");
        let win = HardwareInfo { os: "windows".into(), arch: "x86_64".into(), cuda: true, ..Default::default() };
        let a = pin.select(&win).unwrap();
        assert!(a.name.contains("win-cuda") && a.extra_name.is_some());
        let bsd = HardwareInfo { os: "freebsd".into(), arch: "x86_64".into(), ..Default::default() };
        assert!(pin.select(&bsd).is_none());
    }

    #[test]
    fn locate_prefers_config() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("llama-server");
        std::fs::write(&fake, "x").unwrap();
        let m = LlamaCppManager::new(dir.path());
        assert_eq!(m.locate(fake.to_str().unwrap()), Some(Located::Configured(fake.clone())));
        assert_eq!(m.locate(dir.path().join("missing").to_str().unwrap()), None);
    }
}
