//! Model selection, consent-gated downloads, and runtime startup.

use crate::app::{display_path, App, Options};
use clap::Subcommand;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use veyra_core::config::{ModelMode, ProviderKind};
use veyra_model::hardware::{format_bytes, HardwareInfo};
use veyra_model::openai::OpenAiCompatProvider;
use veyra_model::recommend::{recommend, Placement};
use veyra_model::registry::ModelSpec;
use veyra_model::ModelProvider;
use veyra_runtime::llamacpp::LlamaCppManager;
use veyra_runtime::server::{LlamaServer, ServerOptions};
use veyra_runtime::store::ModelStore;

#[derive(Subcommand)]
pub enum ModelsCmd {
    /// List registry models, what is installed, and what fits this machine.
    List,
    /// Download a model (asks for confirmation unless --yes).
    Pull {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Re-verify an installed model's SHA-256.
    Verify { id: String },
    /// Use a specific model by default (writes ~/.veyra/config.toml).
    Use { id: String },
    /// Go back to automatic model selection.
    Auto,
}

/// How to handle downloads that need consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// Ask on the terminal.
    Ask,
    /// Never download (non-interactive runs); fail with instructions.
    Never,
    /// The user already consented (e.g. `models pull --yes`, or the editor UI).
    Granted,
}

pub struct ModelRuntime {
    pub provider: Option<Arc<dyn ModelProvider>>,
    pub server: Option<LlamaServer>,
    pub spec: Option<ModelSpec>,
    pub context: Option<u32>,
    pub label: String,
}

impl ModelRuntime {
    pub fn none(reason: &str) -> Self {
        Self {
            provider: None,
            server: None,
            spec: None,
            context: None,
            label: reason.to_string(),
        }
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut s) = self.server.take() {
            s.stop().await;
        }
    }
}

pub fn confirm(question: &str, default_yes: bool) -> bool {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!(
        "{question} {} ",
        if default_yes { "[Y/n]" } else { "[y/N]" }
    );
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "" => default_yes,
        "y" | "yes" => true,
        _ => false,
    }
}

pub fn describe_download(spec: &ModelSpec, context: u32, store: &ModelStore) {
    let need = spec.memory_needed(context);
    println!(
        "  {:<10} {} ({})",
        "Model",
        style(&spec.name).bold(),
        spec.quantization
    );
    println!(
        "  {:<10} {} (revision {})",
        "Source",
        spec.source_url(),
        &spec.revision[..12]
    );
    println!("  {:<10} {}", "License", spec.license);
    let partial = store.partial_bytes(spec);
    if partial > 0 {
        println!(
            "  {:<10} {} ({} already downloaded, will resume)",
            "Download",
            format_bytes(spec.size_bytes),
            format_bytes(partial)
        );
    } else {
        println!("  {:<10} {}", "Download", format_bytes(spec.size_bytes));
    }
    println!(
        "  {:<10} about {} while running ({context}-token context)",
        "Memory",
        format_bytes(need)
    );
    println!(
        "  {:<10} {}",
        "Saved to",
        display_path(&store.path_for(spec))
    );
    if !spec.notes.is_empty() {
        println!("  {:<10} {}", "Notes", spec.notes);
    }
}

fn progress_bar(total: u64, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "  {msg} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    pb.set_message(label.to_string());
    pb
}

/// Download with a progress bar; Ctrl-C cancels and keeps the partial file.
pub async fn download_model(store: &ModelStore, spec: &ModelSpec) -> anyhow::Result<()> {
    let pb = progress_bar(spec.size_bytes, &spec.name);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let watcher = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            c2.cancel();
        }
    });
    let pb2 = pb.clone();
    let res = store
        .download(spec, &move |p| pb2.set_position(p.downloaded), &cancel)
        .await;
    watcher.abort();
    match &res {
        Ok(_) => pb.finish_with_message(format!("{} (sha256 verified)", spec.name)),
        Err(_) => pb.abandon(),
    }
    res.map(|_| ())
}

pub async fn install_llama(
    app: &App,
    hw: &HardwareInfo,
    consent: Consent,
) -> anyhow::Result<std::path::PathBuf> {
    let mgr = LlamaCppManager::new(&app.paths.runtimes_dir());
    if let Some(found) = mgr.locate(&app.config.runtime.llama_server_path) {
        return Ok(found.binary().to_path_buf());
    }
    let asset = mgr
        .pin
        .select(hw)
        .ok_or_else(|| anyhow::anyhow!("no prebuilt llama.cpp for {}/{}; install llama-server and set runtime.llama_server_path", hw.os, hw.arch))?;
    let total = asset.size + asset.extra_size.unwrap_or(0);
    match consent {
        Consent::Never => anyhow::bail!(
            "llama.cpp is not installed. Run `veyra` interactively or `veyra models pull <id>` to install it."
        ),
        Consent::Ask => {
            println!(
                "\nVeyra runs models with llama.cpp. No `llama-server` was found, so Veyra can install\nthe official prebuilt build {} ({}, {}, MIT license) from github.com/{}.",
                mgr.pin.tag,
                asset.name,
                format_bytes(total),
                mgr.pin.repo
            );
            if !confirm("Download and install it now?", true) {
                anyhow::bail!("llama.cpp is required for local models; set runtime.llama_server_path or use another provider");
            }
        }
        Consent::Granted => {}
    }
    let pb = progress_bar(total, "llama.cpp");
    let pb2 = pb.clone();
    let rec = mgr
        .install(
            hw,
            &move |_, p| pb2.set_position(p.downloaded),
            &CancellationToken::new(),
        )
        .await;
    match rec {
        Ok(r) => {
            pb.finish_with_message(format!("llama.cpp {} (sha256 verified)", r.tag));
            Ok(r.binary)
        }
        Err(e) => {
            pb.abandon();
            Err(e)
        }
    }
}

/// Resolve which registry model to use and with what context/placement.
pub fn choose_model(
    app: &App,
    hw: &HardwareInfo,
    requested: Option<&str>,
) -> anyhow::Result<(ModelSpec, u32, Option<Placement>)> {
    let rec = recommend(&app.models, hw);
    let id = match requested {
        Some(id) if id != "auto" => Some(id.to_string()),
        Some(_) => None,
        None if app.config.model.mode == ModelMode::Manual && !app.config.model.id.is_empty() => {
            Some(app.config.model.id.clone())
        }
        None => None,
    };
    let (spec, ctx, placement) = match id {
        Some(id) => {
            let spec = app
                .models
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown model `{id}`; see `veyra models`"))?
                .clone();
            let cand = rec.candidates.iter().find(|c| c.id == spec.id);
            let ctx = cand
                .filter(|c| c.fits)
                .map(|c| c.context)
                .unwrap_or(spec.default_context);
            (spec, ctx, cand.and_then(|c| c.placement.clone()))
        }
        None => {
            let spec = rec
                .model
                .clone()
                .ok_or_else(|| anyhow::anyhow!("{}", rec.summary))?;
            (spec, rec.context, rec.placement.clone())
        }
    };
    let ctx = if app.config.model.context_length > 0 {
        app.config.model.context_length
    } else {
        ctx
    };
    Ok((spec, ctx, placement))
}

/// Start the configured model runtime.
pub async fn start_runtime(
    app: &App,
    consent: Consent,
    requested: Option<&str>,
) -> anyhow::Result<ModelRuntime> {
    // External OpenAI-compatible runtimes (Ollama, LM Studio, vLLM, ...).
    if app.config.model.provider != ProviderKind::Llamacpp && requested.is_none() {
        let endpoint = app.config.endpoint().ok_or_else(|| {
            anyhow::anyhow!(
                "model.endpoint is required for provider {:?}",
                app.config.model.provider
            )
        })?;
        let key = (!app.config.model.api_key_env.is_empty())
            .then(|| std::env::var(&app.config.model.api_key_env).ok())
            .flatten();
        let mut provider = OpenAiCompatProvider::new(&endpoint, &app.config.model.api_model)
            .with_api_key(key.clone())
            .with_label(app.config.model.provider.label());
        let mut model = app.config.model.api_model.clone();
        if model.is_empty() {
            let models = provider.list_models().await.map_err(|e| {
                anyhow::anyhow!(
                    "cannot reach {} at {endpoint}: {e}",
                    app.config.model.provider.label()
                )
            })?;
            model = models.first().cloned().ok_or_else(|| {
                anyhow::anyhow!("{endpoint} serves no models; set model.api_model")
            })?;
            provider = OpenAiCompatProvider::new(&endpoint, &model)
                .with_api_key(key)
                .with_label(app.config.model.provider.label());
        }
        let ctx = (app.config.model.context_length > 0)
            .then_some(app.config.model.context_length)
            .or(Some(32768));
        let provider = provider.with_context_length(ctx);
        return Ok(ModelRuntime {
            label: format!(
                "{model} ({}, {endpoint})",
                app.config.model.provider.label()
            ),
            provider: Some(Arc::new(provider)),
            server: None,
            spec: None,
            context: ctx,
        });
    }

    let hw = tokio::task::spawn_blocking({
        let dir = app.paths.models_dir();
        move || HardwareInfo::detect(&dir)
    })
    .await?;
    let (spec, ctx, placement) = choose_model(app, &hw, requested)?;
    let store = ModelStore::new(&app.paths.models_dir());
    if !store.is_installed(&spec) {
        match consent {
            Consent::Never => anyhow::bail!(
                "model {} is not downloaded. Run `veyra models pull {}` (about {}).",
                spec.name,
                spec.id,
                format_bytes(spec.size_bytes)
            ),
            Consent::Ask => {
                println!("\nVeyra needs a local model. Recommended for this machine:");
                describe_download(&spec, ctx, &store);
                if spec.size_bytes > 20_000_000_000 {
                    println!(
                        "  {}",
                        style("This is a large download (over 20 GB).").yellow()
                    );
                }
                if !confirm("Download it now?", false) {
                    return Ok(ModelRuntime::none("no model (download declined)"));
                }
            }
            Consent::Granted => {}
        }
        download_model(&store, &spec).await?;
    }
    let binary = install_llama(app, &hw, consent).await?;
    let gpu = hw.metal || hw.cuda || hw.rocm || hw.vulkan;
    let opts = ServerOptions {
        binary,
        model_path: store.path_for(&spec),
        host: app.config.runtime.bind_host.clone(),
        context: ctx,
        gpu_layers: if gpu {
            app.config.runtime.gpu_layers
        } else {
            0
        },
        cpu_moe: placement == Some(Placement::PartialOffload),
        alias: spec.id.clone(),
        reasoning: app.config.model.reasoning.clone(),
        reasoning_budget: if app.config.model.reasoning_budget != 0 {
            app.config.model.reasoning_budget
        } else {
            spec.reasoning_budget
        },
        extra_args: app.config.runtime.extra_args.clone(),
        log_file: app.paths.logs_dir().join("llama-server.log"),
        startup_timeout: Duration::from_secs(app.config.runtime.startup_timeout_secs),
    };
    let server = LlamaServer::start(opts).await?;
    let provider = OpenAiCompatProvider::new(&server.base_url, &spec.id)
        .with_label("llama.cpp")
        .with_context_length(Some(ctx));
    Ok(ModelRuntime {
        label: format!(
            "{} {} (llama.cpp, {}, {}k context)",
            spec.name,
            spec.quantization,
            server.base_url.trim_end_matches("/v1"),
            ctx / 1024
        ),
        provider: Some(Arc::new(provider)),
        server: Some(server),
        spec: Some(spec),
        context: Some(ctx),
    })
}

/// Set `[model] mode = "manual"` / `id = "..."` (or back to auto) in the
/// config text, preserving everything else including comments.
pub fn set_model_in_config(text: &str, id: Option<&str>) -> String {
    let mut out = Vec::new();
    let mut in_model = false;
    let mut wrote = false;
    let mode = if id.is_some() { "manual" } else { "auto" };
    let push_settings = |out: &mut Vec<String>| {
        out.push(format!("mode = \"{mode}\""));
        if let Some(id) = id {
            out.push(format!("id = \"{id}\""));
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if in_model && !wrote {
                push_settings(&mut out);
                wrote = true;
            }
            in_model = t == "[model]";
            out.push(line.to_string());
            continue;
        }
        if in_model
            && (t.starts_with("mode") || t.starts_with("id ") || t.starts_with("id="))
            && t.contains('=')
        {
            if !wrote {
                push_settings(&mut out);
                wrote = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if in_model && !wrote {
        push_settings(&mut out);
        wrote = true;
    }
    if !wrote {
        out.push(String::new());
        out.push("[model]".into());
        push_settings(&mut out);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub fn print_list(app: &App) {
    let store = ModelStore::new(&app.paths.models_dir());
    let hw = HardwareInfo::detect(&app.paths.models_dir());
    let rec = recommend(&app.models, &hw);
    println!("{}", style("Models").bold());
    for m in &app.models.models {
        let c = rec.candidates.iter().find(|c| c.id == m.id);
        let installed = store.is_installed(m);
        let mark = if rec.model.as_ref().map(|r| r.id == m.id).unwrap_or(false) {
            style("★ recommended").green().to_string()
        } else if c.map(|c| c.fits).unwrap_or(false) {
            style("fits").cyan().to_string()
        } else {
            style("too large").dim().to_string()
        };
        println!(
            "  {:<26} {:>8}  {:<9} {:<12} {}{}",
            m.id,
            format_bytes(m.size_bytes),
            m.tier,
            m.license,
            mark,
            if installed {
                style("  installed").green().to_string()
            } else {
                String::new()
            }
        );
        if let Some(c) = c {
            println!("  {:<26} {}", "", style(&c.reason).dim());
        }
    }
    println!("\n{}", rec.summary);
}

pub fn run(
    rt: &tokio::runtime::Runtime,
    opts: &Options,
    action: Option<ModelsCmd>,
) -> anyhow::Result<i32> {
    let app = App::load(opts)?;
    let store = ModelStore::new(&app.paths.models_dir());
    match action.unwrap_or(ModelsCmd::List) {
        ModelsCmd::List => {
            print_list(&app);
            Ok(0)
        }
        ModelsCmd::Pull { id, yes } => {
            let spec = app
                .models
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown model `{id}`"))?
                .clone();
            if store.is_installed(&spec) {
                println!(
                    "{} is already installed at {}",
                    spec.name,
                    display_path(&store.path_for(&spec))
                );
                return Ok(0);
            }
            describe_download(&spec, spec.default_context, &store);
            if !yes && !confirm("Download it now?", false) {
                println!("Cancelled.");
                return Ok(1);
            }
            rt.block_on(download_model(&store, &spec))?;
            let hw = HardwareInfo::detect(&app.paths.models_dir());
            rt.block_on(install_llama(
                &app,
                &hw,
                if yes { Consent::Granted } else { Consent::Ask },
            ))?;
            Ok(0)
        }
        ModelsCmd::Verify { id } => {
            let spec = app
                .models
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown model `{id}`"))?;
            if !store.path_for(spec).exists() {
                anyhow::bail!("{} is not downloaded", spec.name);
            }
            print!("Hashing {}… ", display_path(&store.path_for(spec)));
            let _ = std::io::stdout().flush();
            if store.verify(spec)? {
                println!("{}", style("sha256 OK").green());
                Ok(0)
            } else {
                println!(
                    "{}",
                    style("sha256 MISMATCH: delete the file and download again").red()
                );
                Ok(1)
            }
        }
        ModelsCmd::Use { id } => {
            let spec = app
                .models
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("unknown model `{id}`"))?;
            let f = app.paths.config_file();
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            std::fs::write(&f, set_model_in_config(&text, Some(&spec.id)))?;
            println!("Default model set to {} in {}", spec.name, display_path(&f));
            Ok(0)
        }
        ModelsCmd::Auto => {
            let f = app.paths.config_file();
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            std::fs::write(&f, set_model_in_config(&text, None))?;
            println!("Model selection set to auto in {}", display_path(&f));
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_model_edit_preserves_comments() {
        let text = veyra_core::config::DEFAULT_CONFIG_TOML;
        let edited = set_model_in_config(text, Some("qwen3-4b-q4_k_m"));
        assert!(edited.contains("# Veyra configuration"));
        let c = veyra_core::Config::parse(&edited).unwrap();
        assert_eq!(c.model.mode, ModelMode::Manual);
        assert_eq!(c.model.id, "qwen3-4b-q4_k_m");
        let back = set_model_in_config(&edited, None);
        let c = veyra_core::Config::parse(&back).unwrap();
        assert_eq!(c.model.mode, ModelMode::Auto);
        assert!(c.model.id.is_empty());
        let fresh = set_model_in_config("", Some("x"));
        assert_eq!(veyra_core::Config::parse(&fresh).unwrap().model.id, "x");
    }
}
