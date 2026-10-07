//! Resolve where inference comes from.
//!
//! Kara Core asks this module for an [`InferenceSession`] and gets back a
//! provider behind the [`InferenceProvider`] interface, or no provider plus
//! guidance. Nothing outside this crate knows whether inference runs on this
//! machine, on another Kara machine, or behind an OpenAI-compatible endpoint.
//!
//! Hardware-adaptive behaviour: local inference is optional. When the machine
//! cannot run a useful local model, or the user declines a download, Kara
//! keeps working as an agent client and recommends compute the user owns
//! elsewhere. Nothing is downloaded without explicit consent.

use crate::local::hardware::{format_bytes, HardwareInfo};
use crate::local::llamacpp::LlamaCppManager;
use crate::local::recommend::{recommend, Placement};
use crate::local::registry::{ModelSpec, Registry};
use crate::local::server::{reap_stale, LlamaServer, ServerOptions};
use crate::local::store::ModelStore;
use crate::openai::OpenAiCompatProvider;
use crate::InferenceProvider;
use kara_core::config::{Config, ProviderKind};
use kara_core::KaraPaths;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// How downloads that need consent are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// Ask through the [`SetupUi`].
    Ask,
    /// Never download (non-interactive runs, editor start-up).
    Never,
    /// The user already consented (e.g. `kara models pull --yes`, or a
    /// confirmed choice in the editor).
    Granted,
}

/// What the user is asked before a model download.
#[derive(Debug, Clone)]
pub struct DownloadOffer {
    pub name: String,
    pub quantization: String,
    pub size_bytes: u64,
    pub partial_bytes: u64,
    pub memory_needed: u64,
    pub context: u32,
    pub license: String,
    pub source: String,
    pub revision: String,
    pub path: std::path::PathBuf,
    pub notes: String,
}

/// What the user is asked before installing a local runtime.
#[derive(Debug, Clone)]
pub struct RuntimeOffer {
    pub name: String,
    pub version: String,
    pub asset: String,
    pub size_bytes: u64,
    pub license: String,
    pub source: String,
}

/// Front-end hooks for consent and progress (terminal, editor, silent).
pub trait SetupUi: Send + Sync {
    fn confirm_model_download(&self, offer: &DownloadOffer) -> bool;
    fn confirm_runtime_install(&self, offer: &RuntimeOffer) -> bool;
    fn progress(&self, label: &str, done: u64, total: Option<u64>);
    fn progress_done(&self, label: &str, ok: bool);
    fn notice(&self, message: &str);
}

/// Declines everything and reports nothing.
pub struct SilentUi;

impl SetupUi for SilentUi {
    fn confirm_model_download(&self, _: &DownloadOffer) -> bool {
        false
    }
    fn confirm_runtime_install(&self, _: &RuntimeOffer) -> bool {
        false
    }
    fn progress(&self, _: &str, _: u64, _: Option<u64>) {}
    fn progress_done(&self, _: &str, _: bool) {}
    fn notice(&self, _: &str) {}
}

/// The resolved inference backend for a session.
pub struct InferenceSession {
    pub provider: Option<Arc<dyn InferenceProvider>>,
    pub kind: ProviderKind,
    /// Human-readable description, e.g. "Qwen3-4B Q4_K_M (local, 32k context)".
    pub label: String,
    pub context: Option<u32>,
    /// Registry model when inference is local.
    pub local_model: Option<ModelSpec>,
    /// OpenAI-compatible base URL serving this session (for `kara serve --inference`).
    pub upstream: Option<String>,
    pub upstream_key: Option<String>,
    /// When no provider is available: why, and what the user can do.
    pub guidance: Option<String>,
    server: Option<LlamaServer>,
}

impl InferenceSession {
    pub fn none(guidance: impl Into<String>) -> Self {
        Self {
            provider: None,
            kind: ProviderKind::Local,
            label: "none".into(),
            context: None,
            local_model: None,
            upstream: None,
            upstream_key: None,
            guidance: Some(guidance.into()),
            server: None,
        }
    }

    pub fn is_available(&self) -> bool {
        self.provider.is_some()
    }

    /// PID of a local runtime process, if one was started.
    pub fn runtime_pid(&self) -> Option<u32> {
        self.server.as_ref().and_then(|s| s.pid())
    }

    pub fn runtime_log(&self) -> Option<&std::path::Path> {
        self.server.as_ref().map(|s| s.log_file.as_path())
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut s) = self.server.take() {
            s.stop().await;
        }
    }
}

/// How to continue without local inference.
pub fn remote_options() -> String {
    "Kara works without local inference. Point it at compute you own:\n  \
     kara connect http://<host>:7878            another machine running `kara serve --inference`\n  \
     kara config set inference.provider ollama   or lmstudio / vllm / openai_compat (with inference.endpoint)"
        .to_string()
}

/// Bearer token for an endpoint: `api_key_env`, else `api_key_file`.
pub fn endpoint_key(config: &Config) -> Option<String> {
    let inf = &config.inference;
    if !inf.api_key_env.is_empty() {
        if let Ok(v) = std::env::var(&inf.api_key_env) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    if !inf.api_key_file.is_empty() {
        return std::fs::read_to_string(&inf.api_key_file)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    None
}

pub struct ResolveRequest<'a> {
    pub config: &'a Config,
    pub paths: &'a KaraPaths,
    pub registry: &'a Registry,
    pub consent: Consent,
    pub ui: &'a dyn SetupUi,
    /// Overrides the configured source: "auto", a registry id (local), or
    /// `None` for the configuration's choice.
    pub model_override: Option<&'a str>,
}

/// Resolve the configured inference source. Errors are only returned for
/// real failures (e.g. a runtime that will not start); "no inference
/// available" is a normal outcome with guidance.
pub async fn resolve(req: ResolveRequest<'_>) -> anyhow::Result<InferenceSession> {
    if req.config.inference.provider.is_endpoint() && req.model_override.is_none() {
        return Ok(resolve_endpoint(req.config).await);
    }
    resolve_local(req).await
}

async fn resolve_endpoint(config: &Config) -> InferenceSession {
    let kind = config.inference.provider;
    let Some(endpoint) = config.endpoint() else {
        return InferenceSession::none(format!(
            "inference.provider = \"{}\" needs an endpoint: kara config set inference.endpoint <url>",
            kind.as_str()
        ));
    };
    let key = endpoint_key(config);
    let wanted = config.inference.model.trim();
    let mut model = if wanted == "auto" {
        String::new()
    } else {
        wanted.to_string()
    };
    let probe = OpenAiCompatProvider::new(&endpoint, &model).with_api_key(key.clone());
    match probe.list_models().await {
        Ok(models) => {
            if model.is_empty() {
                match models.first() {
                    Some(m) => model = m.clone(),
                    None => {
                        return InferenceSession::none(format!(
                        "{endpoint} serves no models. Load a model there, or set inference.model."
                    ))
                    }
                }
            }
        }
        Err(e) => {
            if model.is_empty() {
                return InferenceSession::none(format!(
                    "cannot reach the {} at {endpoint}: {e}\n{}",
                    kind.label(),
                    remote_options()
                ));
            }
            // Some servers do not implement /models; try anyway.
        }
    }
    let ctx = Some(if config.inference.context_length > 0 {
        config.inference.context_length
    } else {
        32768
    });
    let provider = OpenAiCompatProvider::new(&endpoint, &model)
        .with_api_key(key.clone())
        .with_label(kind.label())
        .with_context_length(ctx);
    InferenceSession {
        provider: Some(Arc::new(provider)),
        kind,
        label: format!("{model} ({}, {endpoint})", kind.label()),
        context: ctx,
        local_model: None,
        upstream: Some(endpoint),
        upstream_key: key,
        guidance: None,
        server: None,
    }
}

/// Choose a registry model for this machine (or the requested one).
pub fn choose_model(
    config: &Config,
    registry: &Registry,
    hw: &HardwareInfo,
    requested: Option<&str>,
) -> Result<(ModelSpec, u32, Option<Placement>), String> {
    let rec = recommend(registry, hw);
    let configured = config.inference.model.trim();
    let id = match requested {
        Some(id) if id != "auto" => Some(id.to_string()),
        Some(_) => None,
        None if !configured.is_empty() && configured != "auto" => Some(configured.to_string()),
        None => None,
    };
    let (spec, ctx, placement) = match id {
        Some(id) => {
            let spec = registry
                .get(&id)
                .ok_or_else(|| format!("unknown model `{id}`; see `kara models`"))?
                .clone();
            let cand = rec.candidates.iter().find(|c| c.id == spec.id);
            let ctx = cand
                .filter(|c| c.fits)
                .map(|c| c.context)
                .unwrap_or(spec.default_context);
            (spec, ctx, cand.and_then(|c| c.placement.clone()))
        }
        None => match rec.model.clone() {
            Some(spec) => (spec, rec.context, rec.placement.clone()),
            None => {
                return Err(format!(
                    "This machine has about {} available for models, which is not enough for a useful local model.\n{}",
                    format_bytes(rec.budget),
                    remote_options()
                ))
            }
        },
    };
    let ctx = if config.inference.context_length > 0 {
        config.inference.context_length
    } else {
        ctx
    };
    Ok((spec, ctx, placement))
}

pub fn download_offer(spec: &ModelSpec, context: u32, store: &ModelStore) -> DownloadOffer {
    DownloadOffer {
        name: spec.name.clone(),
        quantization: spec.quantization.clone(),
        size_bytes: spec.size_bytes,
        partial_bytes: store.partial_bytes(spec),
        memory_needed: spec.memory_needed(context),
        context,
        license: spec.license.clone(),
        source: spec.source_url(),
        revision: spec.revision.clone(),
        path: store.path_for(spec),
        notes: spec.notes.clone(),
    }
}

/// Download a model with progress through the UI.
pub async fn download_model(
    store: &ModelStore,
    spec: &ModelSpec,
    ui: &dyn SetupUi,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let label = spec.name.clone();
    let res = store
        .download(
            spec,
            &|p| ui.progress(&label, p.downloaded, p.total),
            cancel,
        )
        .await;
    ui.progress_done(&label, res.is_ok());
    res.map(|_| ())
}

/// Locate the local runtime, installing it after consent if missing.
pub async fn ensure_runtime(
    config: &Config,
    paths: &KaraPaths,
    hw: &HardwareInfo,
    consent: Consent,
    ui: &dyn SetupUi,
) -> anyhow::Result<std::path::PathBuf> {
    let mgr = LlamaCppManager::new(&paths.runtimes_dir());
    if let Some(found) = mgr.locate(&config.inference.local.server_path) {
        return Ok(found.binary().to_path_buf());
    }
    let asset = mgr.pin.select(hw).ok_or_else(|| {
        anyhow::anyhow!(
            "no prebuilt local runtime for {}/{}; install llama-server and set inference.local.server_path, or use another inference provider",
            hw.os,
            hw.arch
        )
    })?;
    let offer = RuntimeOffer {
        name: "llama.cpp".into(),
        version: mgr.pin.tag.clone(),
        asset: asset.name.clone(),
        size_bytes: asset.size + asset.extra_size.unwrap_or(0),
        license: mgr.pin.license.clone(),
        source: format!("https://github.com/{}", mgr.pin.repo),
    };
    let ok = match consent {
        Consent::Never => false,
        Consent::Granted => true,
        Consent::Ask => ui.confirm_runtime_install(&offer),
    };
    if !ok {
        anyhow::bail!(
            "the local runtime is not installed. Run `kara models pull <id>` to install it, or use another inference provider.\n{}",
            remote_options()
        );
    }
    let label = format!("{} {}", offer.name, offer.version);
    let rec = mgr
        .install(
            hw,
            &|_, p| ui.progress(&label, p.downloaded, p.total),
            &CancellationToken::new(),
        )
        .await;
    ui.progress_done(&label, rec.is_ok());
    Ok(rec?.binary)
}

/// `None` if `available` has enough headroom over `needed` to load safely
/// right now; otherwise the guidance message to show instead of loading.
/// 20% headroom beyond the model's own estimate, floor 512MB, so a close
/// call still refuses rather than guessing.
fn memory_gate_message(model_id: &str, needed: u64, available: u64) -> Option<String> {
    let headroom = (needed / 5).max(512 << 20);
    let required = needed.saturating_add(headroom);
    if available >= required {
        return None;
    }
    Some(format!(
        "{model_id} needs about {} free to load safely right now, but only {} is available \
         (another program — possibly another Kara session — may already be using it). \
         Close something and try again, or `kara config set inference.model auto` to let \
         Kara pick a smaller model for this machine.",
        format_bytes(required),
        format_bytes(available),
    ))
}

async fn resolve_local(req: ResolveRequest<'_>) -> anyhow::Result<InferenceSession> {
    let models_dir = req.paths.models_dir();
    let hw = tokio::task::spawn_blocking(move || HardwareInfo::detect(&models_dir)).await?;
    let (spec, ctx, placement) =
        match choose_model(req.config, req.registry, &hw, req.model_override) {
            Ok(x) => x,
            Err(guidance) => return Ok(InferenceSession::none(guidance)),
        };
    let store = ModelStore::new(&req.paths.models_dir());
    if !store.is_installed(&spec) {
        let offer = download_offer(&spec, ctx, &store);
        let ok = match req.consent {
            Consent::Never => false,
            Consent::Granted => true,
            Consent::Ask => req.ui.confirm_model_download(&offer),
        };
        if !ok {
            return Ok(InferenceSession::none(format!(
                "No local model is installed. To use one on this machine: kara models pull {} ({}).\n{}",
                spec.id,
                format_bytes(spec.size_bytes),
                remote_options()
            )));
        }
        download_model(&store, &spec, req.ui, &CancellationToken::new()).await?;
    }

    // Reuse an already-running server for this exact model instead of
    // loading a second full copy into RAM. This is what actually turns a
    // "fine on its own" machine into a frozen one: two Kara clients
    // starting close together (two windows, the CLI plus the desktop app,
    // a restart racing a still-shutting-down previous run) each loading
    // their own multi-GB copy of the same model at once.
    if let Some(base_url) = crate::local::server::find_running(&req.paths.logs_dir(), &store.path_for(&spec)).await {
        let provider = OpenAiCompatProvider::new(&base_url, &spec.id)
            .with_label("local")
            .with_context_length(Some(ctx));
        return Ok(InferenceSession {
            provider: Some(Arc::new(provider)),
            kind: ProviderKind::Local,
            label: format!(
                "{} {} (local, {}k context, shared with another Kara session)",
                spec.name,
                spec.quantization,
                ctx / 1024
            ),
            context: Some(ctx),
            local_model: Some(spec),
            upstream: Some(base_url),
            upstream_key: None,
            guidance: None,
            server: None,
        });
    }

    // Live safety gate, re-checked on every start — not just when a model is
    // first recommended. A model picked in an earlier session (or `--model`)
    // skips the recommender's own fit check entirely, which is how a
    // machine under memory pressure right now (another Kara process already
    // holding a model in RAM, another app, swap) still got a fresh
    // multi-GB `llama-server` thrown at it and hung.
    if let Some(msg) = memory_gate_message(&spec.id, spec.memory_needed(ctx), hw.available_ram) {
        return Ok(InferenceSession::none(msg));
    }

    let binary = ensure_runtime(req.config, req.paths, &hw, req.consent, req.ui).await?;
    let local = &req.config.inference.local;
    let accelerated = hw.metal || hw.cuda || hw.rocm || hw.vulkan;
    let opts = ServerOptions {
        binary,
        model_path: store.path_for(&spec),
        host: local.bind_host.clone(),
        context: ctx,
        gpu_layers: if accelerated { local.gpu_layers } else { 0 },
        cpu_moe: placement == Some(Placement::PartialOffload),
        alias: spec.id.clone(),
        reasoning: req.config.inference.reasoning.clone(),
        reasoning_budget: if req.config.inference.reasoning_budget != 0 {
            req.config.inference.reasoning_budget
        } else {
            spec.reasoning_budget
        },
        extra_args: local.extra_args.clone(),
        log_file: req.paths.logs_dir().join("local-runtime.log"),
        startup_timeout: Duration::from_secs(local.startup_timeout_secs),
    };
    let reaped = reap_stale(&req.paths.logs_dir());
    if reaped > 0 {
        req.ui.notice(&format!(
            "stopped {reaped} local runtime process(es) left behind by an earlier Kara run"
        ));
    }
    let server = LlamaServer::start(opts).await?;
    let provider = OpenAiCompatProvider::new(&server.base_url, &spec.id)
        .with_label("local")
        .with_context_length(Some(ctx));
    Ok(InferenceSession {
        provider: Some(Arc::new(provider)),
        kind: ProviderKind::Local,
        label: format!(
            "{} {} (local, {}k context)",
            spec.name,
            spec.quantization,
            ctx / 1024
        ),
        context: Some(ctx),
        local_model: Some(spec),
        upstream: Some(server.base_url.clone()),
        upstream_key: None,
        guidance: None,
        server: Some(server),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::hardware::HardwareInfo;

    fn tiny() -> HardwareInfo {
        HardwareInfo {
            total_ram: 4 << 30,
            available_ram: 2 << 30,
            ..Default::default()
        }
    }

    #[test]
    fn memory_gate_refuses_a_close_call_instead_of_guessing() {
        let one_gb = 1u64 << 30;
        // Comfortably clears needed + 20% headroom.
        assert!(memory_gate_message("m", one_gb, one_gb * 2).is_none());
        // Technically more than `needed`, but inside the headroom: refuse.
        let msg = memory_gate_message("m", one_gb, one_gb + (one_gb / 10)).unwrap();
        assert!(msg.contains("another Kara session"), "{msg}");
        assert!(msg.contains("kara config set inference.model auto"), "{msg}");
    }

    #[test]
    fn memory_gate_floor_covers_tiny_models_too() {
        // A tiny model's 20% headroom could round to a few MB; the 512MB
        // floor is what actually protects a low-RAM machine from a close
        // call on a 100MB model.
        let tiny_model = 100u64 << 20;
        assert!(memory_gate_message("m", tiny_model, tiny_model + (400 << 20)).is_some());
        assert!(memory_gate_message("m", tiny_model, tiny_model + (600 << 20)).is_none());
    }

    #[test]
    fn weak_machine_gets_guidance_not_an_error() {
        let err =
            choose_model(&Config::default(), &Registry::builtin(), &tiny(), None).unwrap_err();
        assert!(err.contains("not enough for a useful local model"), "{err}");
        assert!(err.contains("kara connect"), "{err}");
    }

    #[tokio::test]
    async fn weak_machine_resolves_to_no_inference_and_downloads_nothing() {
        // KARA_HOME-style paths in a temp dir; the real machine is used for
        // detection, so force an impossible model instead of relying on RAM.
        let d = tempfile::tempdir().unwrap();
        let paths = KaraPaths::at(d.path());
        let mut config = Config::default();
        config.inference.model = "qwen3-4b-q4_k_m".into();
        let s = resolve(ResolveRequest {
            config: &config,
            paths: &paths,
            registry: &Registry::builtin(),
            consent: Consent::Never,
            ui: &SilentUi,
            model_override: None,
        })
        .await
        .unwrap();
        assert!(!s.is_available());
        assert!(s.guidance.unwrap().contains("kara models pull"));
        assert!(
            std::fs::read_dir(d.path()).map(|r| r.count()).unwrap_or(0) == 0
                || !paths.models_dir().join("ggml-org__Qwen3-4B-GGUF").exists()
        );
    }

    #[tokio::test]
    async fn unreachable_endpoint_gives_guidance() {
        let mut config = Config::default();
        config.inference.provider = ProviderKind::OpenaiCompat;
        config.inference.endpoint = "http://127.0.0.1:9/v1".into();
        let d = tempfile::tempdir().unwrap();
        let s = resolve(ResolveRequest {
            config: &config,
            paths: &KaraPaths::at(d.path()),
            registry: &Registry::builtin(),
            consent: Consent::Never,
            ui: &SilentUi,
            model_override: None,
        })
        .await
        .unwrap();
        assert!(!s.is_available());
        assert!(s.guidance.unwrap().contains("cannot reach"));
    }

    #[test]
    fn endpoint_key_from_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("token");
        std::fs::write(&f, "abc\n").unwrap();
        let mut c = Config::default();
        c.inference.api_key_file = f.display().to_string();
        assert_eq!(endpoint_key(&c).as_deref(), Some("abc"));
    }
}
