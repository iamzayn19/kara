//! `kara`: the local AI engineering agent.

mod app;
mod commands;
mod doctor;
mod evalcmd;
mod models;
mod render;
mod serve;
mod share;
mod tui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "kara",
    version,
    about = "Kara: your code, your machine, your AI. A free, local AI engineering agent.",
    long_about = "Kara is a free, open-source local AI engineering agent.\n\nRun `kara` inside a repository and describe what you want in plain language.\nKara imposes no usage limits; it runs as much as your hardware can handle."
)]
struct Cli {
    /// Repository to work in (default: the git root containing the current directory).
    #[arg(short = 'C', long, global = true)]
    dir: Option<std::path::PathBuf>,

    /// Permission mode for this session: ask, workspace or full.
    #[arg(long, global = true)]
    permissions: Option<String>,

    /// Where inference runs for this session: local, kara, openai_compat, ollama, lmstudio or vllm.
    #[arg(long, global = true)]
    provider: Option<String>,

    /// Model for this session (local model id, `auto`, or an endpoint's model name).
    #[arg(long, global = true)]
    model: Option<String>,

    /// Resume the most recent session for this repository.
    #[arg(long)]
    resume: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run one task non-interactively and print the result.
    Run {
        /// The task, in plain language.
        prompt: Vec<String>,
        /// Approve ordinary permission requests automatically (high-risk
        /// actions are still denied).
        #[arg(long)]
        yes: bool,
        /// Emit agent events as JSON lines.
        #[arg(long)]
        json: bool,
        /// Plan only (Morpheus): investigate and propose, change nothing.
        #[arg(long)]
        plan: bool,
        /// Review the current diff (Oracle).
        #[arg(long, conflicts_with = "plan")]
        review: bool,
    },
    /// Detect hardware, runtimes and models, and recommend a model.
    Doctor,
    /// Show what leaves this machine (nothing, by default).
    Privacy,
    /// Manage local models.
    Models {
        #[command(subcommand)]
        action: Option<models::ModelsCmd>,
    },
    /// Run the evaluation suite against a model.
    Eval(evalcmd::EvalArgs),
    /// Undo Kara's most recent change batch in this repository.
    Undo,
    /// List recent sessions.
    Sessions,
    /// Speak JSON-RPC over stdio (used by the VS Code extension).
    /// Serve an editor over stdio, or share this machine's inference.
    Serve {
        /// Speak JSON-RPC over stdin/stdout (used by editors; no port is opened).
        #[arg(long, conflicts_with = "inference")]
        stdio: bool,
        /// Share this machine's inference with your other Kara machines.
        #[arg(long)]
        inference: bool,
        /// Address for --inference (default 127.0.0.1:7878).
        #[arg(long, requires = "inference")]
        listen: Option<String>,
    },
    /// Use inference from another Kara machine you own.
    Connect {
        /// URL or host of the machine running `kara serve --inference`.
        url: String,
        /// Token printed by that machine (or KARA_REMOTE_TOKEN).
        #[arg(long)]
        token: Option<String>,
    },
    /// Show or initialize configuration.
    Config {
        #[command(subcommand)]
        action: Option<ConfigCmd>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the effective value of a key, e.g. `permissions.mode`.
    Get { key: String },
    /// Set a key in your user config, e.g. `permissions.mode workspace`.
    Set { key: String, value: String },
    /// Print the config file path.
    Path,
    /// Print the effective configuration.
    Show,
    /// Write the default config file if none exists.
    Init,
}

fn main() {
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    install_signal_handlers(&rt);
    let code = match run(cli, &rt) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{} {e:#}", console::style("error:").red().bold());
            1
        }
    };
    rt.shutdown_timeout(std::time::Duration::from_secs(2));
    std::process::exit(code);
}

/// Stop any llama-server this process started when Kara is terminated, so
/// the model runtime never outlives it.
fn install_signal_handlers(rt: &tokio::runtime::Runtime) {
    #[cfg(unix)]
    rt.spawn(async {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = hup.recv() => {}
        }
        kara_inference::local::stop_all_runtimes();
        std::process::exit(143);
    });
    #[cfg(windows)]
    rt.spawn(async {
        if let Ok(mut close) = tokio::signal::windows::ctrl_close() {
            close.recv().await;
            kara_inference::local::stop_all_runtimes();
            std::process::exit(1);
        }
    });
}

fn run(cli: Cli, rt: &tokio::runtime::Runtime) -> anyhow::Result<i32> {
    let opts = app::Options {
        dir: cli.dir.clone(),
        permissions: cli.permissions.clone(),
        provider: cli.provider.clone(),
        model: cli.model.clone(),
    };
    match cli.command {
        None => tui::interactive(rt, &opts, cli.resume),
        Some(Command::Run {
            prompt,
            yes,
            json,
            plan,
            review,
        }) => {
            let mode = if plan {
                kara_protocol::AgentMode::Plan
            } else if review {
                kara_protocol::AgentMode::Review
            } else {
                kara_protocol::AgentMode::Execute
            };
            tui::run_once(rt, &opts, &prompt.join(" "), mode, yes, json)
        }
        Some(Command::Doctor) => doctor::doctor(rt, &opts),
        Some(Command::Privacy) => doctor::privacy(&opts),
        Some(Command::Models { action }) => models::run(rt, &opts, action),
        Some(Command::Eval(args)) => evalcmd::run(rt, &opts, args),
        Some(Command::Undo) => tui::undo_cli(&opts),
        Some(Command::Sessions) => tui::sessions_cli(&opts),
        Some(Command::Serve {
            stdio,
            inference,
            listen,
        }) => {
            if inference {
                share::serve_inference(rt, &opts, listen)
            } else if stdio {
                serve::serve_stdio(rt, &opts)
            } else {
                anyhow::bail!("use `kara serve --stdio` (editors) or `kara serve --inference` (share inference)")
            }
        }
        Some(Command::Connect { url, token }) => share::connect(rt, &url, token),
        Some(Command::Config { action }) => {
            let paths = kara_core::KaraPaths::discover()?;
            match action.unwrap_or(ConfigCmd::Show) {
                ConfigCmd::Path => println!("{}", paths.config_file().display()),
                ConfigCmd::Get { key } => {
                    let root = app::workspace_root(&opts).ok();
                    let loaded = kara_core::Config::load(&paths.config_file(), root.as_deref())?;
                    println!("{}", kara_core::config_edit::get(&loaded.config, &key)?);
                }
                ConfigCmd::Set { key, value } => {
                    app::ensure_user_config(&paths)?;
                    kara_core::config_edit::set_in_file(&paths.config_file(), &key, &value)?;
                    println!("{key} = {value}  ({})", paths.config_file().display());
                    if key == "permissions.mode" && value.trim() == "full" {
                        eprintln!(
                            "note: full mode lets Kara run shell commands, delete files, use the network and commit without asking. Secrets, sudo, git push, destructive commands and paths outside the workspace still ask."
                        );
                    }
                }
                ConfigCmd::Init => {
                    app::ensure_user_config(&paths)?;
                    println!("{}", paths.config_file().display());
                }
                ConfigCmd::Show => {
                    let root = app::workspace_root(&opts).ok();
                    let loaded = kara_core::Config::load(&paths.config_file(), root.as_deref())?;
                    for w in &loaded.warnings {
                        eprintln!("warning: {w}");
                    }
                    print!("{}", loaded.config.to_toml());
                }
            }
            Ok(0)
        }
    }
}
