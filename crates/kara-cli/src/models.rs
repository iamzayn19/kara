//! Terminal front end for inference setup: consent prompts, progress bars and
//! the `kara models` commands. Resolution itself lives in `kara-inference`.

use crate::app::{display_path, App, Options};
use clap::Subcommand;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use kara_inference::local::hardware::format_bytes;
use kara_inference::source::{
    self, Consent, DownloadOffer, InferenceSession, ResolveRequest, RuntimeOffer, SetupUi,
};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Subcommand)]
pub enum ModelsCmd {
    /// List local models, what is installed, and what fits this machine.
    List,
    /// Download a model for local inference (asks for confirmation unless --yes).
    Pull {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Re-verify an installed model's SHA-256.
    Verify { id: String },
    /// Use a specific local model by default.
    Use { id: String },
    /// Pick the local model automatically for this machine.
    Auto,
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

pub fn describe_download(o: &DownloadOffer) {
    println!(
        "  {:<10} {} ({})",
        "Model",
        style(&o.name).bold(),
        o.quantization
    );
    println!(
        "  {:<10} {} (revision {})",
        "Source",
        o.source,
        &o.revision[..12.min(o.revision.len())]
    );
    println!("  {:<10} {}", "License", o.license);
    if o.partial_bytes > 0 {
        println!(
            "  {:<10} {} ({} already downloaded, will resume)",
            "Download",
            format_bytes(o.size_bytes),
            format_bytes(o.partial_bytes)
        );
    } else {
        println!("  {:<10} {}", "Download", format_bytes(o.size_bytes));
    }
    println!(
        "  {:<10} about {} while running ({}-token context)",
        "Memory",
        format_bytes(o.memory_needed),
        o.context
    );
    println!("  {:<10} {}", "Saved to", display_path(&o.path));
    if !o.notes.is_empty() {
        println!("  {:<10} {}", "Notes", o.notes);
    }
}

/// Prompts on the terminal, progress bars on stderr.
#[derive(Default)]
pub struct TerminalUi {
    bars: Mutex<HashMap<String, ProgressBar>>,
}

impl SetupUi for TerminalUi {
    fn confirm_model_download(&self, o: &DownloadOffer) -> bool {
        println!("\nLocal inference is optional. A model that fits this machine:");
        describe_download(o);
        if o.size_bytes > 20_000_000_000 {
            println!(
                "  {}",
                style("This is a large download (over 20 GB).").yellow()
            );
        }
        println!(
            "  {}",
            style("Or skip this and use another Kara machine or an endpoint (`kara connect`).")
                .dim()
        );
        confirm("Download it now?", false)
    }

    fn confirm_runtime_install(&self, o: &RuntimeOffer) -> bool {
        println!(
            "\nLocal inference uses {}. Kara can install the official prebuilt build {} ({}, {}, {} license) from {}.",
            o.name,
            o.version,
            o.asset,
            format_bytes(o.size_bytes),
            o.license,
            o.source
        );
        confirm("Download and install it now?", true)
    }

    fn progress(&self, label: &str, done: u64, total: Option<u64>) {
        let mut bars = self.bars.lock().unwrap();
        let pb = bars.entry(label.to_string()).or_insert_with(|| {
            let pb = ProgressBar::new(total.unwrap_or(0));
            pb.set_style(
                ProgressStyle::with_template(
                    "  {msg} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
                )
                .unwrap()
                .progress_chars("=> "),
            );
            pb.set_message(label.to_string());
            pb
        });
        if let Some(t) = total {
            pb.set_length(t);
        }
        pb.set_position(done);
    }

    fn progress_done(&self, label: &str, ok: bool) {
        if let Some(pb) = self.bars.lock().unwrap().remove(label) {
            if ok {
                pb.finish_with_message(format!("{label} (sha256 verified)"));
            } else {
                pb.abandon();
            }
        }
    }

    fn notice(&self, message: &str) {
        eprintln!("{}", style(message).dim());
    }
}

/// Resolve inference for this app. Never fails just because no inference is
/// available: the session then carries guidance instead of a provider.
pub async fn start_inference(
    app: &App,
    consent: Consent,
    model_override: Option<&str>,
    ui: &dyn SetupUi,
) -> anyhow::Result<InferenceSession> {
    source::resolve(ResolveRequest {
        config: &app.config,
        paths: &app.paths,
        registry: &app.models,
        consent,
        ui,
        model_override,
    })
    .await
}

pub fn print_list(app: &App) {
    let (entries, summary) = kara_inference::local::catalog(&app.paths, &app.models);
    println!("{}", style("Local models (optional)").bold());
    for e in &entries {
        let mark = if e.recommended {
            style("★ recommended").green().to_string()
        } else if e.fits {
            style("fits").cyan().to_string()
        } else {
            style("too large").dim().to_string()
        };
        println!(
            "  {:<26} {:>8}  {:<9} {:<12} {}{}",
            e.spec.id,
            format_bytes(e.spec.size_bytes),
            e.spec.tier,
            e.spec.license,
            mark,
            if e.installed {
                style("  installed").green().to_string()
            } else {
                String::new()
            }
        );
        if !e.reason.is_empty() {
            println!("  {:<26} {}", "", style(&e.reason).dim());
        }
    }
    println!("\n{summary}");
}

pub fn run(
    rt: &tokio::runtime::Runtime,
    opts: &Options,
    action: Option<ModelsCmd>,
) -> anyhow::Result<i32> {
    let app = App::load(opts)?;
    let spec_for = |id: &str| {
        app.models
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown model `{id}`; see `kara models`"))
    };
    match action.unwrap_or(ModelsCmd::List) {
        ModelsCmd::List => {
            print_list(&app);
            Ok(0)
        }
        ModelsCmd::Pull { id, yes } => {
            let spec = spec_for(&id)?;
            let ui = TerminalUi::default();
            if kara_inference::local::is_installed(&app.paths, &spec) {
                println!(
                    "{} is already installed at {}",
                    spec.name,
                    display_path(&kara_inference::local::model_path(&app.paths, &spec))
                );
            } else {
                let store = kara_inference::local::store::ModelStore::new(&app.paths.models_dir());
                let offer = source::download_offer(&spec, spec.default_context, &store);
                describe_download(&offer);
                if !yes && !confirm("Download it now?", false) {
                    println!("Cancelled.");
                    return Ok(1);
                }
                let cancel = CancellationToken::new();
                let c2 = cancel.clone();
                let watcher = rt.spawn(async move {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        c2.cancel();
                    }
                });
                let res = rt.block_on(source::download_model(&store, &spec, &ui, &cancel));
                watcher.abort();
                res?;
            }
            let hw = kara_inference::local::hardware::HardwareInfo::detect(&app.paths.models_dir());
            rt.block_on(source::ensure_runtime(
                &app.config,
                &app.paths,
                &hw,
                if yes { Consent::Granted } else { Consent::Ask },
                &ui,
            ))?;
            Ok(0)
        }
        ModelsCmd::Verify { id } => {
            let spec = spec_for(&id)?;
            let path = kara_inference::local::model_path(&app.paths, &spec);
            if !path.exists() {
                anyhow::bail!("{} is not downloaded", spec.name);
            }
            print!("Hashing {}… ", display_path(&path));
            let _ = std::io::stdout().flush();
            if kara_inference::local::verify_model(&app.paths, &spec)? {
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
            let spec = spec_for(&id)?;
            let f = app.paths.config_file();
            kara_core::config_edit::set_in_file(&f, "inference.provider", "local")?;
            kara_core::config_edit::set_in_file(&f, "inference.model", &spec.id)?;
            println!("Default model set to {} in {}", spec.name, display_path(&f));
            Ok(0)
        }
        ModelsCmd::Auto => {
            let f = app.paths.config_file();
            kara_core::config_edit::set_in_file(&f, "inference.model", "auto")?;
            println!("Local model selection set to auto in {}", display_path(&f));
            Ok(0)
        }
    }
}
