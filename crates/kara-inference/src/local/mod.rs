//! Optional local inference backend.
//!
//! Used only when the user chooses local inference. Kara runs without it.
//!
//! * [`hardware`]: detect RAM, CPU architecture, accelerators and disk.
//! * [`registry`] / [`recommend`]: which model, if any, suits this machine.
//! * [`download`]: resumable HTTPS downloads verified against SHA-256.
//! * [`archive`]: safe extraction (no path traversal) of runtime archives.
//! * [`llamacpp`]: locate or install a pinned llama.cpp build.
//! * [`server`]: launch `llama-server` on 127.0.0.1, wait for health, stop it.
//! * [`store`]: where models live and what is recorded about them.
//!
//! Network access here is limited to software and model downloads that the
//! user approved. Inference itself stays on the local machine.

pub mod archive;
pub mod download;
pub mod hardware;
pub mod llamacpp;
pub mod recommend;
pub mod registry;
pub mod server;
pub mod store;

pub const USER_AGENT: &str = concat!(
    "kara/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/iamzayn19/kara)"
);

use hardware::HardwareInfo;
use recommend::{recommend, Recommendation};
use registry::{ModelSpec, Registry};
use serde::Serialize;
use store::{ModelRecord, ModelStore};

/// Stop every local runtime process this Kara process started.
pub fn stop_all_runtimes() {
    server::kill_all_started();
}

/// Snapshot of local inference capability, for `kara doctor` and editors.
#[derive(Debug, Clone, Serialize)]
pub struct LocalStatus {
    pub hardware: HardwareInfo,
    pub memory_budget: u64,
    /// Where the local runtime is, if installed.
    pub runtime: Option<String>,
    pub runtime_version: Option<String>,
    /// Prebuilt runtime that would be installed for this host.
    pub runtime_available: Option<String>,
    pub installed: Vec<ModelRecord>,
    pub recommendation: Recommendation,
}

pub fn status(
    config: &kara_core::Config,
    paths: &kara_core::KaraPaths,
    registry: &Registry,
) -> LocalStatus {
    let hw = HardwareInfo::detect(&paths.models_dir());
    let mgr = llamacpp::LlamaCppManager::new(&paths.runtimes_dir());
    let found = mgr.locate(&config.inference.local.server_path);
    LocalStatus {
        memory_budget: hw.fast_memory_budget(),
        runtime: found.as_ref().map(|f| f.describe()),
        runtime_version: found.as_ref().and_then(|f| llamacpp::version(f.binary())),
        runtime_available: mgr.pin.select(&hw).map(|a| a.name.clone()),
        installed: ModelStore::new(&paths.models_dir()).installed(),
        recommendation: recommend(registry, &hw),
        hardware: hw,
    }
}

/// One registry entry as seen from this machine.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub spec: ModelSpec,
    pub installed: bool,
    pub fits: bool,
    pub recommended: bool,
    pub reason: String,
    pub memory_needed: u64,
}

pub fn catalog(paths: &kara_core::KaraPaths, registry: &Registry) -> (Vec<CatalogEntry>, String) {
    let hw = HardwareInfo::detect(&paths.models_dir());
    let rec = recommend(registry, &hw);
    let store = ModelStore::new(&paths.models_dir());
    let entries = registry
        .models
        .iter()
        .map(|m| {
            let c = rec.candidates.iter().find(|c| c.id == m.id);
            CatalogEntry {
                installed: store.is_installed(m),
                fits: c.map(|c| c.fits).unwrap_or(false),
                recommended: rec.model.as_ref().map(|r| r.id == m.id).unwrap_or(false),
                reason: c.map(|c| c.reason.clone()).unwrap_or_default(),
                memory_needed: c.map(|c| c.memory_needed).unwrap_or(m.size_bytes),
                spec: m.clone(),
            }
        })
        .collect();
    let summary = match &rec.model {
        Some(_) => rec.summary.clone(),
        None => format!("{}\n{}", rec.summary, crate::source::remote_options()),
    };
    (entries, summary)
}

/// Re-hash an installed model against its pinned SHA-256.
pub fn verify_model(paths: &kara_core::KaraPaths, spec: &ModelSpec) -> anyhow::Result<bool> {
    ModelStore::new(&paths.models_dir()).verify(spec)
}

pub fn model_path(paths: &kara_core::KaraPaths, spec: &ModelSpec) -> std::path::PathBuf {
    ModelStore::new(&paths.models_dir()).path_for(spec)
}

pub fn is_installed(paths: &kara_core::KaraPaths, spec: &ModelSpec) -> bool {
    ModelStore::new(&paths.models_dir()).is_installed(spec)
}
