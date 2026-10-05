//! `veyra`: the local AI engineering agent.

mod app;
mod commands;
mod doctor;
mod evalcmd;
mod models;
mod render;
mod serve;
mod tui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "veyra",
    version,
    about = "Veyra: your code, your machine, your AI. A free, local AI engineering agent.",
    long_about = "Veyra is a free, open-source local AI engineering agent.\n\nRun `veyra` inside a repository and describe what you want in plain language.\nVeyra imposes no usage limits; it runs as much as your hardware can handle."
)]
struct Cli {
    /// Repository to work in (default: the git root containing the current directory).
    #[arg(short = 'C', long, global = true)]
    dir: Option<std::path::PathBuf>,

    /// Permission profile for this run: safe, balanced or autonomous.
    #[arg(long, global = true)]
    profile: Option<String>,

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
    /// Undo Veyra's most recent change batch in this repository.
    Undo,
    /// List recent sessions.
    Sessions,
    /// Speak JSON-RPC over stdio (used by the VS Code extension).
    Serve {
        /// Required: communicate over stdin/stdout. No network port is opened.
        #[arg(long)]
        stdio: bool,
    },
    /// Show or initialize configuration.
    Config {
        #[command(subcommand)]
        action: Option<ConfigCmd>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
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

fn run(cli: Cli, rt: &tokio::runtime::Runtime) -> anyhow::Result<i32> {
    let opts = app::Options {
        dir: cli.dir.clone(),
        profile: cli.profile.clone(),
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
                veyra_protocol::AgentMode::Plan
            } else if review {
                veyra_protocol::AgentMode::Review
            } else {
                veyra_protocol::AgentMode::Execute
            };
            tui::run_once(rt, &opts, &prompt.join(" "), mode, yes, json)
        }
        Some(Command::Doctor) => doctor::doctor(rt, &opts),
        Some(Command::Privacy) => doctor::privacy(&opts),
        Some(Command::Models { action }) => models::run(rt, &opts, action),
        Some(Command::Eval(args)) => evalcmd::run(rt, &opts, args),
        Some(Command::Undo) => tui::undo_cli(&opts),
        Some(Command::Sessions) => tui::sessions_cli(&opts),
        Some(Command::Serve { stdio }) => {
            if !stdio {
                anyhow::bail!("only `veyra serve --stdio` is supported; Veyra never opens a network port for editors");
            }
            serve::serve_stdio(rt, &opts)
        }
        Some(Command::Config { action }) => {
            let paths = veyra_core::VeyraPaths::discover()?;
            match action.unwrap_or(ConfigCmd::Show) {
                ConfigCmd::Path => println!("{}", paths.config_file().display()),
                ConfigCmd::Init => {
                    app::ensure_user_config(&paths)?;
                    println!("{}", paths.config_file().display());
                }
                ConfigCmd::Show => {
                    let root = app::workspace_root(&opts).ok();
                    let loaded = veyra_core::Config::load(&paths.config_file(), root.as_deref())?;
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
