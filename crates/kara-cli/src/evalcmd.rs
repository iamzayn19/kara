//! `kara eval`: run the agent evaluation suite against a model.

use crate::app::{App, Options};
use crate::models::{self, TerminalUi};
use clap::Args;
use console::style;
use kara_agent::eval::{self, EvalReport};
use kara_inference::source::Consent;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Args)]
pub struct EvalArgs {
    /// Directory of fixture repositories (default: tests/fixtures/repos in this repository).
    #[arg(long)]
    suite: Option<PathBuf>,
    /// Only run these task ids (comma-separated), or tasks whose id starts with a prefix ending in `*`.
    #[arg(long)]
    tasks: Option<String>,
    /// Replay reference solutions instead of a model. Validates fixtures and
    /// the harness; says nothing about model quality.
    #[arg(long)]
    oracle: bool,
    /// Allow downloading the model if needed.
    #[arg(long)]
    yes: bool,
    /// Wall-clock limit per task in seconds (0 = none). Bounds the benchmark,
    /// not the agent.
    #[arg(long, default_value_t = 1800)]
    task_timeout: u64,
    /// Write the JSON report here (default: tests/evals/results/<date>-<model>.json when the directory exists).
    #[arg(long)]
    out: Option<PathBuf>,
}

fn select(tasks: Vec<eval::EvalTask>, filter: &Option<String>) -> Vec<eval::EvalTask> {
    let Some(f) = filter else { return tasks };
    let wanted: Vec<&str> = f
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    tasks
        .into_iter()
        .filter(|t| {
            wanted.iter().any(|w| match w.strip_suffix('*') {
                Some(prefix) => t.id.starts_with(prefix),
                None => t.id == *w,
            })
        })
        .collect()
}

pub fn run(rt: &tokio::runtime::Runtime, opts: &Options, args: EvalArgs) -> anyhow::Result<i32> {
    let app = App::load(opts)?;
    let suite = args
        .suite
        .clone()
        .unwrap_or_else(|| app.root.join("tests/fixtures/repos"));
    if !suite.exists() {
        anyhow::bail!("suite {} not found; pass --suite <dir>", suite.display());
    }
    let tasks = select(eval::load_suite(&suite)?, &args.tasks);
    if tasks.is_empty() {
        anyhow::bail!("no tasks selected");
    }

    let mut runtime = None;
    let (model_name, provider_label) = if args.oracle {
        (
            "oracle (reference solutions)".to_string(),
            "scripted".to_string(),
        )
    } else {
        let consent = if args.yes {
            Consent::Granted
        } else {
            Consent::Ask
        };
        let ui = TerminalUi::default();
        let r = rt.block_on(models::start_inference(
            &app,
            consent,
            opts.model.as_deref(),
            &ui,
        ))?;
        if r.provider.is_none() {
            anyhow::bail!(
                "no inference available to evaluate.\n{}",
                r.guidance.clone().unwrap_or_default()
            );
        }
        let provider_label = r.kind.as_str().to_string();
        let name = r
            .local_model
            .as_ref()
            .map(|s| format!("{} {}", s.name, s.quantization))
            .unwrap_or_else(|| r.label.clone());
        runtime = Some(r);
        (name, provider_label)
    };

    let server_pid = runtime.as_ref().and_then(|r| r.runtime_pid());
    let sampler: Option<Arc<dyn Fn() -> Option<u64> + Send + Sync>> = server_pid.map(|pid| {
        let f: Arc<dyn Fn() -> Option<u64> + Send + Sync> = Arc::new(move || {
            let mut sys = sysinfo::System::new();
            let p = sysinfo::Pid::from_u32(pid);
            sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
            sys.process(p).map(|pr| pr.memory())
        });
        f
    });

    println!(
        "{} {} task(s) with {}",
        style("Evaluating").bold(),
        tasks.len(),
        style(&model_name).cyan()
    );
    let mut report = EvalReport {
        model: model_name.clone(),
        provider: provider_label,
        started: chrono::Utc::now().to_rfc3339(),
        kara_version: kara_core::VERSION.into(),
        host: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        results: Vec::new(),
    };
    let out = args.out.clone().or_else(|| {
        let dir = app.root.join("tests/evals/results");
        dir.exists().then(|| {
            let slug: String = model_name
                .to_ascii_lowercase()
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '.' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            dir.join(format!("{}-{slug}.json", &report.started[..10]))
        })
    });
    for t in &tasks {
        print!("  {:<28} ", t.id);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let provider: Arc<dyn kara_inference::InferenceProvider> = if args.oracle {
            Arc::new(eval::oracle_provider(t)?)
        } else {
            runtime
                .as_ref()
                .and_then(|r| r.provider.clone())
                .expect("provider")
        };
        let settings = app.agent_settings(runtime.as_ref().and_then(|r| r.context));
        let r = rt.block_on(eval::run_task(
            t,
            provider,
            settings,
            sampler.clone(),
            (args.task_timeout > 0).then(|| std::time::Duration::from_secs(args.task_timeout)),
        ));
        if let Some(why) = &r.skipped {
            println!(
                "{} {}",
                style("skipped").yellow(),
                style(format!("({why})")).dim()
            );
            report.results.push(r);
            continue;
        }
        let mark = if r.success {
            style("solved").green()
        } else {
            style("failed").red()
        };
        println!(
            "{mark} {}",
            style(format!(
                "({} model calls, {} tool calls, {:.1}s{})",
                r.iterations,
                r.tool_calls,
                r.duration_ms as f64 / 1000.0,
                r.error
                    .as_ref()
                    .map(|e| format!(", {}", e.chars().take(80).collect::<String>()))
                    .unwrap_or_default()
            ))
            .dim()
        );
        report.results.push(r);
        // Write after every task so long runs keep partial results.
        if let Some(p) = &out {
            let _ = std::fs::write(p, serde_json::to_vec_pretty(&report).unwrap_or_default());
        }
    }
    if let Some(mut r) = runtime {
        rt.block_on(r.shutdown());
    }

    println!("\n{}", report.markdown());
    if let Some(p) = out {
        std::fs::write(&p, serde_json::to_vec_pretty(&report)?)?;
        println!("Report written to {}", p.display());
    }
    Ok(if report.solved() == report.attempted() {
        0
    } else {
        2
    })
}
