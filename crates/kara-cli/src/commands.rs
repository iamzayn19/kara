//! Slash commands. Natural language is the primary interface; these are
//! shortcuts and controls.

use crate::models::{self, TerminalUi};
use crate::render::colorize_diff;
use crate::tui::Session;
use console::style;
use kara_core::permissions::{Mode, PolicyDecision};
use kara_inference::source::{Consent, InferenceSession};
use kara_protocol::AgentMode;
use kara_tools::{Tool, UndoReport};
use serde_json::json;

pub enum Action {
    Continue,
    Exit,
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("/help", "show commands"),
    (
        "/model",
        "show the model, or /model <id> | /model auto to switch",
    ),
    ("/models", "list models and what fits this machine"),
    ("/status", "session, model and repository status"),
    ("/context", "context window usage and working memory"),
    (
        "/plan",
        "plan a task without changing anything (alias of /morpheus)",
    ),
    (
        "/morpheus",
        "mentor mode: investigate, propose a plan, wait for approval",
    ),
    ("/approve", "carry out the pending plan"),
    ("/diff", "show Kara's changes, separately from your own"),
    ("/test", "run the project's tests (optionally: files)"),
    ("/lint", "run the linter or type checker"),
    ("/build", "run the build"),
    ("/git", "git status with change ownership"),
    ("/review", "review the current diff (alias of /oracle)"),
    ("/oracle", "risk review of the diff by severity"),
    (
        "/matrix",
        "what Kara currently sees: repo, files, symbols, model, state",
    ),
    ("/undo", "undo Kara's most recent change batch"),
    ("/clear", "forget the conversation (files are untouched)"),
    ("/compact", "summarize the conversation to free context"),
    (
        "/permissions",
        "show permissions, or /permissions <ask|workspace|full>",
    ),
    ("/privacy", "what leaves this machine"),
    ("/doctor", "hardware, runtime and model diagnostics"),
    ("/sessions", "recent sessions in this repository"),
    ("/exit", "quit"),
];

fn header(title: &str) {
    println!("{}", style(title).bold());
}

pub fn handle(
    s: &mut Session,
    rt: &tokio::runtime::Runtime,
    input: &str,
) -> anyhow::Result<Action> {
    let (cmd, arg) = match input.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (input, ""),
    };
    match cmd {
        "/help" | "/?" => {
            header(
                "Talk to Kara in plain language, e.g. \"fix the failing auth tests\". Commands:",
            );
            for (name, desc) in COMMANDS {
                println!("  {:<13} {}", style(name).cyan(), desc);
            }
            println!(
                "  {}",
                style("End a line with \\ for multi-line input.").dim()
            );
        }
        "/exit" | "/quit" | "/q" => return Ok(Action::Exit),
        "/model" => model_cmd(s, rt, arg)?,
        "/models" => models::print_list(&s.app),
        "/status" => status(s),
        "/context" => context(s),
        "/plan" | "/morpheus" => {
            if arg.is_empty() {
                match s.agent.pending_plan() {
                    Some((task, plan)) => {
                        header(&format!("Pending plan for: {task}"));
                        println!("{plan}\n\n{}", style("Type /approve to carry it out.").cyan());
                    }
                    None => println!("Usage: {cmd} <task>. Kara investigates and proposes a plan; nothing changes until you /approve."),
                }
            } else if s.inference.provider.is_none() {
                println!("{}", style(crate::tui::no_model_message()).yellow());
            } else {
                println!(
                    "{}",
                    style("Planning (read-only). Nothing will change until you approve.").dim()
                );
                s.turn(rt, arg, AgentMode::Plan);
            }
        }
        "/approve" => {
            if s.agent.pending_plan().is_none() {
                println!("No plan is waiting for approval. Use /morpheus <task> first.");
            } else {
                s.approve(rt);
            }
        }
        "/review" | "/oracle" => {
            if s.inference.provider.is_none() {
                println!("{}", style(crate::tui::no_model_message()).yellow());
            } else {
                let task = if arg.is_empty() {
                    "Review my current diff. Report regressions, missing tests, suspicious logic, security implications, runtime failures and blast radius by severity.".to_string()
                } else {
                    format!("Review my current diff, focusing on: {arg}")
                };
                println!("{}", style("Reviewing (read-only)…").dim());
                s.turn(rt, &task, AgentMode::Review);
            }
        }
        "/diff" => diff(s),
        "/test" => {
            let files: Vec<String> = arg.split_whitespace().map(str::to_string).collect();
            let args = if files.is_empty() {
                json!({})
            } else {
                json!({"files": files})
            };
            direct_tool(s, rt, &kara_tools::exec::RunTest, args);
        }
        "/lint" => {
            let args = if arg == "typecheck" {
                json!({"kind": "typecheck"})
            } else {
                json!({})
            };
            direct_tool(s, rt, &kara_tools::exec::RunLint, args);
        }
        "/build" => direct_tool(s, rt, &kara_tools::exec::RunBuild, json!({})),
        "/git" => direct_tool(s, rt, &kara_tools::gittools::GitStatus, json!({})),
        "/undo" => {
            let report = s.agent.ctx.journal().undo(None);
            match report {
                Ok(r) => print_undo(&r),
                Err(e) => println!("{e}"),
            }
        }
        "/clear" => {
            s.agent.clear();
            println!("Conversation cleared. Files and the undo history are unchanged.");
        }
        "/compact" => {
            let (before, after) = s.agent.compact_history();
            println!("Conversation compacted: ~{before} → ~{after} tokens.");
        }
        "/permissions" => permissions(s, arg)?,
        "/privacy" => crate::doctor::print_privacy(&s.app.config),
        "/doctor" => crate::doctor::print_doctor(&s.app, rt),
        "/sessions" => {
            for info in s.app.sessions.list(Some(&s.app.root), 10)? {
                let current = if info.id == s.session_id {
                    " (current)"
                } else {
                    ""
                };
                println!(
                    "  {}  {}  {:>3} turns  {}{current}",
                    &info.id[..8],
                    info.updated.get(..16).unwrap_or(""),
                    info.turns,
                    info.title
                );
            }
        }
        "/matrix" => matrix(s),
        other => println!("Unknown command {other}. Type /help."),
    }
    Ok(Action::Continue)
}

fn model_cmd(s: &mut Session, rt: &tokio::runtime::Runtime, arg: &str) -> anyhow::Result<()> {
    if arg.is_empty() {
        header("Inference");
        if s.inference.provider.is_some() {
            println!("  {}", s.model_label());
            if let Some(spec) = &s.inference.local_model {
                println!(
                    "  {} {} ({}), license {}",
                    style("source:").dim(),
                    spec.source_url(),
                    &spec.revision[..12],
                    spec.license
                );
            }
            if let Some(log) = s.inference.runtime_log() {
                println!(
                    "  {} pid {:?}, log {}",
                    style("local runtime:").dim(),
                    s.inference.runtime_pid(),
                    crate::app::display_path(log)
                );
            }
        } else {
            println!(
                "  none\n{}",
                s.inference
                    .guidance
                    .clone()
                    .unwrap_or_else(kara_inference::source::remote_options)
            );
        }
        return Ok(());
    }
    let requested = if arg == "auto" { "auto" } else { arg };
    println!("{}", style("Starting model…").dim());
    // Stop the current runtime first to free memory for the new one.
    rt.block_on(s.inference.shutdown());
    match rt.block_on(models::start_inference(
        &s.app,
        Consent::Ask,
        Some(requested),
        &TerminalUi::default(),
    )) {
        Ok(new) => {
            let label = new.label.clone();
            let has = new.provider.is_some();
            s.set_inference(rt, new);
            if has {
                println!("Now using {label}");
            } else {
                println!(
                    "No inference started.\n{}",
                    s.inference.guidance.clone().unwrap_or_default()
                );
            }
        }
        Err(e) => {
            println!("{} {e:#}", style("could not start model:").red());
            s.set_inference(rt, InferenceSession::none(format!("{e:#}")));
        }
    }
    Ok(())
}

fn status(s: &Session) {
    header("Status");
    println!(
        "  {:<13}{}",
        "model",
        if s.inference.provider.is_some() {
            s.model_label()
        } else {
            "none".into()
        }
    );
    println!("  {:<13}{}", "repository", s.app.root.display());
    println!("  {:<13}{}", "permissions", s.agent.policy.mode.as_str());
    println!("  {:<13}{}", "session", s.session_id);
    let turns = s
        .app
        .sessions
        .turns(&s.session_id)
        .map(|t| t.len())
        .unwrap_or(0);
    println!(
        "  {:<13}{turns} turns, {} tokens in / {} out",
        "usage", s.agent.total_usage.prompt_tokens, s.agent.total_usage.completion_tokens
    );
    let changed = s.agent.ctx.journal().kara_changed_paths();
    println!(
        "  {:<13}{}",
        "kara edits",
        if changed.is_empty() {
            "none".to_string()
        } else {
            changed.into_iter().collect::<Vec<_>>().join(", ")
        }
    );
    if let Ok(st) = s.app.index.stats() {
        println!(
            "  {:<13}{} files, {} symbols",
            "index", st.files, st.symbols
        );
    }
    if let Some((task, _)) = s.agent.pending_plan() {
        println!("  {:<13}plan awaiting approval: {task}", "agent");
    } else {
        println!("  {:<13}idle", "agent");
    }
}

fn context(s: &Session) {
    header("Context");
    let window = s.agent.settings.context_window as usize;
    let used = s.agent.history_tokens();
    println!(
        "  conversation history ~{used} tokens of a {window}-token window ({:.0}%)",
        used as f64 * 100.0 / window.max(1) as f64
    );
    println!("  {}", style("Each request adds the system prompt, repository orientation and working memory; older tool output is elided automatically.").dim());
    if !s.agent.state.task.is_empty() {
        println!();
        print!("{}", s.agent.state.render());
    }
}

fn diff(s: &Session) {
    let j = s.agent.ctx.journal();
    let kara = j.kara_changed_paths();
    let root = s.app.root.clone();
    if kara.is_empty() {
        println!("Kara has not changed any files in this session.");
    } else {
        header("Changes made by Kara");
        for path in &kara {
            let before_bytes = j.original_content(path).flatten().unwrap_or_default();
            let after_bytes = std::fs::read(root.join(path)).unwrap_or_default();
            if kara_context::looks_binary(&before_bytes) || kara_context::looks_binary(&after_bytes)
            {
                println!(
                    "{} {path} {}",
                    style("binary").yellow(),
                    style(format!(
                        "({} → {} bytes)",
                        before_bytes.len(),
                        after_bytes.len()
                    ))
                    .dim()
                );
                continue;
            }
            let before = String::from_utf8_lossy(&before_bytes).into_owned();
            let after = String::from_utf8_lossy(&after_bytes).into_owned();
            print!(
                "{}",
                colorize_diff(&kara_tools::patch::unified_diff(path, &before, &after))
            );
        }
    }
    let user: Vec<String> = kara_context::git::dirty_paths(&root)
        .into_iter()
        .filter(|p| !kara.contains(p))
        .collect();
    if !user.is_empty() {
        println!();
        header("Your uncommitted changes (not made by Kara)");
        for p in user {
            println!("  {p}");
        }
    }
}

fn direct_tool(
    s: &Session,
    rt: &tokio::runtime::Runtime,
    tool: &dyn Tool,
    args: serde_json::Value,
) {
    // The user typed the command, so it is authorized; hard limits still apply.
    match tool.assess(&args, &s.agent.ctx) {
        Ok(a) => {
            if let Some(b) = a.blocked {
                println!("{} {b}", style("blocked:").red());
                return;
            }
            println!("{} {}", style("→").cyan(), a.title);
        }
        Err(e) => {
            println!("{e}");
            return;
        }
    }
    let out = rt.block_on(tool.run(&args, &s.agent.ctx));
    if let Some(t) = &out.test {
        s.renderer
            .handle(&kara_protocol::AgentEvent::TestFinished { report: t.clone() });
        if !t.succeeded() {
            println!(
                "{}",
                style("Ask Kara to fix the failures, e.g. \"fix the failing tests\".").dim()
            );
        }
    } else {
        println!("{}", out.content.trim_end());
    }
}

fn permissions(s: &mut Session, arg: &str) -> anyhow::Result<()> {
    if !arg.is_empty() {
        let p = Mode::parse(arg).ok_or_else(|| {
            anyhow::anyhow!("unknown permission mode `{arg}` (ask, workspace, full)")
        })?;
        s.agent.policy.set_mode(p);
        println!(
            "Permission mode for this session: {} ({})",
            p.as_str(),
            p.describe()
        );
        return Ok(());
    }
    let p = s.agent.policy.mode;
    header(&format!("Permissions: {} ({})", p.as_str(), p.describe()));
    for (kind, decision, granted) in s.agent.policy.table() {
        let d = match decision {
            PolicyDecision::Allow => style("allow").green(),
            PolicyDecision::Ask => style("ask").yellow(),
            PolicyDecision::Deny => style("deny").red(),
        };
        let note = if kind.is_hard_boundary() {
            style(" (always asked individually)").dim().to_string()
        } else if granted {
            style(" (allowed for this session)").dim().to_string()
        } else {
            String::new()
        };
        println!("  {:<18} {d}{note}", kind.label());
    }
    println!("  {}", style("Change for this session with /permissions <ask|workspace|full>; set the default with `kara config set permissions.mode <mode>`.").dim());
    Ok(())
}

fn matrix(s: &Session) {
    let g = |t: &str| style(t.to_string()).green().bold();
    println!(
        "{}",
        style("┌─ the matrix: what Kara sees right now ─────────────────────────").green()
    );
    let app = &s.app;
    let _ = app.index.refresh();
    if let Ok(st) = app.index.stats() {
        println!(
            "{} {} · {} files · {} lines · {} symbols · {} test files",
            g("│ repository  "),
            app.repo_name(),
            st.files,
            st.lines,
            st.symbols,
            st.test_files
        );
        let langs: Vec<String> = st
            .by_language
            .iter()
            .take(8)
            .map(|(l, n)| format!("{l} {n}"))
            .collect();
        println!(
            "{} {}",
            g("│ languages   "),
            if langs.is_empty() {
                "unrecognized (generic tools only)".into()
            } else {
                langs.join(" · ")
            }
        );
    }
    let state = &s.agent.state;
    let read: Vec<&str> = state
        .files_read
        .iter()
        .map(String::as_str)
        .take(10)
        .collect();
    let changed: Vec<&str> = state
        .files_changed
        .iter()
        .map(String::as_str)
        .take(10)
        .collect();
    println!(
        "{} read: {} · changed: {}",
        g("│ active files"),
        if read.is_empty() {
            "-".into()
        } else {
            read.join(", ")
        },
        if changed.is_empty() {
            "-".into()
        } else {
            changed.join(", ")
        }
    );
    let mut syms = Vec::new();
    for f in state
        .files_changed
        .iter()
        .chain(state.files_read.iter())
        .take(4)
    {
        if let Ok(list) = app.index.symbols_in(f) {
            for sym in list.into_iter().take(4) {
                syms.push(format!(
                    "{} {} ({}:{})",
                    sym.kind, sym.name, sym.path, sym.line
                ));
            }
        }
    }
    println!(
        "{} {}",
        g("│ symbols     "),
        if syms.is_empty() {
            "-".into()
        } else {
            syms.join(", ")
        }
    );
    let kara = s.agent.ctx.journal().kara_changed_paths();
    let user: Vec<String> = kara_context::git::dirty_paths(&app.root)
        .into_iter()
        .filter(|p| !kara.contains(p))
        .collect();
    println!(
        "{} kara: {} · yours: {}",
        g("│ changes     "),
        if kara.is_empty() {
            "-".into()
        } else {
            kara.iter().cloned().collect::<Vec<_>>().join(", ")
        },
        if user.is_empty() {
            "-".into()
        } else {
            user.join(", ")
        }
    );
    println!(
        "{} {}",
        g("│ model       "),
        if s.inference.provider.is_some() {
            s.model_label()
        } else {
            "none".into()
        }
    );
    let window = s.agent.settings.context_window as usize;
    let used = s.agent.history_tokens();
    println!(
        "{} ~{used} / {window} tokens of history ({:.0}%)",
        g("│ context     "),
        used as f64 * 100.0 / window.max(1) as f64
    );
    let tools: Vec<&str> = s.agent.tools().iter().map(|t| t.name()).collect();
    println!(
        "{} {} + update_plan ({} tools)",
        g("│ tools       "),
        tools.join(" "),
        tools.len() + 1
    );
    println!(
        "{} {}",
        g("│ task        "),
        if state.task.is_empty() {
            "-".to_string()
        } else {
            state
                .task
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(100)
                .collect()
        }
    );
    for step in &state.plan {
        println!(
            "{}   {:?}: {}",
            g("│             "),
            step.status,
            step.title
        );
    }
    let agent_state = if s.agent.pending_plan().is_some() {
        "awaiting plan approval"
    } else {
        "idle"
    };
    println!(
        "{} {agent_state} · permissions {}",
        g("│ agent       "),
        s.agent.policy.mode.as_str()
    );
    println!(
        "{}",
        style("└─────────────────────────────────────────────────────────────────").green()
    );
}

pub fn print_undo(r: &UndoReport) {
    if r.restored.is_empty() && r.conflicts.is_empty() && r.errors.is_empty() {
        println!("Nothing needed restoring for batch {}.", r.batch_id);
    }
    for p in &r.restored {
        println!("  {} restored {p}", style("✓").green());
    }
    for p in &r.conflicts {
        println!(
            "  {} kept {p}: it was edited after Kara changed it, so undo left it alone",
            style("!").yellow()
        );
    }
    for e in &r.errors {
        println!("  {} {e}", style("✗").red());
    }
}
