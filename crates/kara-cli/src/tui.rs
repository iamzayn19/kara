//! Interactive terminal session and one-shot runs.

use crate::app::{App, Options};
use crate::commands::{self, Action};
use crate::models::{self, Consent, ModelRuntime};
use crate::render::Renderer;
use console::style;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Editor, Helper};
use std::borrow::Cow;
use std::io::Write;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use kara_agent::approver::{ApproveOrdinary, Approver, DenyAll};
use kara_agent::session::TurnRecord;
use kara_agent::{Agent, TurnResult};
use kara_core::permissions::PermissionPolicy;
use kara_model::{ChatRequest, ChatResponse, EventSink, ModelProvider, ProviderInfo};
use kara_protocol::{AgentMode, PermissionDecision, PermissionRequest};

/// Placeholder provider when no model is running; explains how to get one.
pub struct NoModel(pub String);

#[async_trait::async_trait]
impl ModelProvider for NoModel {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            provider: "none".into(),
            model: "none".into(),
            endpoint: String::new(),
            local: true,
            context_length: None,
        }
    }
    async fn chat(
        &self,
        _: ChatRequest,
        _: EventSink<'_>,
        _: &CancellationToken,
    ) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("{}", self.0)
    }
}

/// Interactive permission prompt.
pub struct TerminalApprover;

#[async_trait::async_trait]
impl Approver for TerminalApprover {
    async fn decide(&self, r: &PermissionRequest) -> PermissionDecision {
        let r = r.clone();
        tokio::task::spawn_blocking(move || prompt_permission(&r))
            .await
            .unwrap_or(PermissionDecision::Deny)
    }
}

fn prompt_permission(r: &PermissionRequest) -> PermissionDecision {
    let kinds: Vec<&str> = r.kinds.iter().map(|k| k.label()).collect();
    let hard = !r.can_remember;
    println!();
    println!(
        "{} {}",
        style(if hard {
            "⚠ approval needed (high risk)"
        } else {
            "? approval needed"
        })
        .yellow()
        .bold(),
        style(&r.title).bold()
    );
    println!("  {} {}", style("category:").dim(), kinds.join(", "));
    for reason in &r.reasons {
        println!("  {} {reason}", style("·").dim());
    }
    let is_diff = r.detail.contains("\n@@") || r.detail.starts_with("---");
    if is_diff {
        let lines = r.detail.lines().count();
        if lines <= 30 {
            print!("{}", crate::render::colorize_diff(&r.detail));
        } else {
            println!(
                "  {} ({lines} lines; press d to view)",
                style("diff preview").dim()
            );
        }
    } else if !r.detail.is_empty() && r.detail != r.title.trim_start_matches("run: ") {
        println!("  {}", r.detail);
    }
    loop {
        if hard {
            print!("  [y] allow once  [n] deny  [d] details  › ");
        } else {
            print!("  [y] allow once  [a] allow for this session  [n] deny  [d] details  › ");
        }
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            return PermissionDecision::Deny;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return PermissionDecision::AllowOnce,
            "a" | "always" if !hard => return PermissionDecision::AllowSession,
            "n" | "no" | "" => return PermissionDecision::Deny,
            "d" | "details" => {
                if is_diff {
                    print!("{}", crate::render::colorize_diff(&r.detail));
                } else {
                    println!("{}", r.detail);
                }
            }
            _ => {}
        }
    }
}

pub struct Session {
    pub app: App,
    pub runtime: ModelRuntime,
    pub agent: Agent,
    pub session_id: String,
    pub renderer: Renderer,
}

impl Session {
    pub fn record(&self, r: &TurnResult, task: &str, started: &str) {
        let rec = TurnRecord {
            turn: r.turn_id,
            task: task.chars().take(2000).collect(),
            mode: format!("{:?}", r.mode).to_lowercase(),
            outcome: format!("{:?}", r.outcome).to_lowercase(),
            summary: r.summary.chars().take(4000).collect(),
            changed_files: r.changed_files.clone(),
            prompt_tokens: r.stats.usage.prompt_tokens,
            completion_tokens: r.stats.usage.completion_tokens,
            started: started.to_string(),
            finished: chrono::Utc::now().to_rfc3339(),
        };
        let _ = self.app.sessions.record_turn(&self.session_id, &rec);
        let (state, history) = self.agent.snapshot();
        let _ = self
            .app
            .sessions
            .save_state(&self.session_id, &state, &history);
    }

    pub fn model_label(&self) -> String {
        self.runtime.label.clone()
    }

    /// Run one turn with Ctrl-C cancellation.
    pub fn turn(
        &mut self,
        rt: &tokio::runtime::Runtime,
        task: &str,
        mode: AgentMode,
    ) -> TurnResult {
        let started = chrono::Utc::now().to_rfc3339();
        let token = CancellationToken::new();
        self.agent.ctx.cancel = token.clone();
        let watcher = rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                token.cancel();
            }
        });
        let result = rt.block_on(self.agent.run_turn(task, mode));
        watcher.abort();
        self.record(&result, task, &started);
        result
    }

    pub fn approve(&mut self, rt: &tokio::runtime::Runtime) -> Option<TurnResult> {
        let (task, _) = self.agent.pending_plan()?.clone();
        let started = chrono::Utc::now().to_rfc3339();
        let token = CancellationToken::new();
        self.agent.ctx.cancel = token.clone();
        let watcher = rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                token.cancel();
            }
        });
        let result = rt.block_on(self.agent.approve_plan());
        watcher.abort();
        if let Some(r) = &result {
            self.record(r, &format!("(approved plan) {task}"), &started);
        }
        result
    }

    pub fn set_runtime(&mut self, rt: &tokio::runtime::Runtime, new: ModelRuntime) {
        let mut old = std::mem::replace(&mut self.runtime, new);
        rt.block_on(old.shutdown());
        let provider: Arc<dyn ModelProvider> = match &self.runtime.provider {
            Some(p) => p.clone(),
            None => Arc::new(NoModel(no_model_message())),
        };
        self.agent.set_provider(provider);
    }
}

pub fn no_model_message() -> String {
    "No model is running. Use /model auto to pick and download one for this machine, /models to see options, or configure Ollama/LM Studio in ~/.kara/config.toml.".into()
}

fn build_session(
    rt: &tokio::runtime::Runtime,
    opts: &Options,
    resume: bool,
    consent: Consent,
    approver: Arc<dyn Approver>,
    renderer: Renderer,
) -> anyhow::Result<Session> {
    let app = App::load(opts)?;
    for w in &app.warnings {
        eprintln!("{} {w}", style("warning:").yellow());
    }
    let mut renderer = renderer;
    renderer.show_reasoning |= app.config.agent.show_reasoning;
    let indexing = app.refresh_index_in_background();
    let runtime = match rt.block_on(models::start_runtime(&app, consent, None)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {e:#}", style("model:").yellow());
            ModelRuntime::none("no model")
        }
    };
    let (session_id, resumed) = app.open_session(resume, &runtime.label)?;
    let ctx = app.tool_context(&session_id)?;
    let provider: Arc<dyn ModelProvider> = match &runtime.provider {
        Some(p) => p.clone(),
        None => Arc::new(NoModel(no_model_message())),
    };
    let r2 = renderer.clone();
    let mut agent = Agent::new(
        provider,
        ctx,
        PermissionPolicy::new(app.config.permissions.profile),
        approver,
        Arc::new(move |e| r2.handle(&e)),
        app.agent_settings(runtime.context),
    );
    if resumed {
        if let Ok(Some((state, history))) = app.sessions.load_state(&session_id) {
            agent.restore(state, history);
        }
    }
    let _ = indexing; // keeps running; the agent refreshes again before each turn
    Ok(Session {
        app,
        runtime,
        agent,
        session_id,
        renderer,
    })
}

fn banner(s: &Session) {
    let app = &s.app;
    println!(
        "{} {}",
        style("Kara").bold().cyan(),
        style(format!(
            "{} · Your code. Your machine. Your AI.",
            kara_core::VERSION
        ))
        .dim()
    );
    let model = if s.runtime.provider.is_some() {
        s.model_label()
    } else {
        style("none (try /model auto)").yellow().to_string()
    };
    println!("{:<13}{}", "Local model:", model);
    let git = if kara_context::git::is_repo(&app.root) {
        let dirty = kara_context::git::dirty_paths(&app.root).len();
        format!(
            "git: {}{}",
            kara_context::git::branch(&app.root).unwrap_or_default(),
            if dirty > 0 {
                format!(", {dirty} uncommitted")
            } else {
                String::new()
            }
        )
    } else {
        "not a git repository".into()
    };
    let mut by_count: Vec<(&String, &usize)> = app.profile.languages.iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(a.1));
    let langs: Vec<&str> = by_count.iter().map(|(l, _)| l.as_str()).take(4).collect();
    println!(
        "{:<13}{} ({git}){}",
        "Repository:",
        app.repo_name(),
        if langs.is_empty() {
            String::new()
        } else {
            format!(" · {}", langs.join(", "))
        }
    );
    println!(
        "{:<13}{} · {}",
        "Permissions:",
        app.config.permissions.profile.as_str(),
        style("/help for commands, Ctrl-C cancels a running task, Ctrl-D exits").dim()
    );
    println!();
}

struct KaraHelper {
    commands: Vec<(&'static str, &'static str)>,
}

impl Completer for KaraHelper {
    type Candidate = Pair;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        if !line.starts_with('/') || line[..pos].contains(' ') {
            return Ok((0, vec![]));
        }
        let prefix = &line[..pos];
        let c = self
            .commands
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, desc)| Pair {
                display: format!("{name:<14} {desc}"),
                replacement: name.to_string(),
            })
            .collect();
        Ok((0, c))
    }
}

impl Hinter for KaraHelper {
    type Hint = String;
    fn hint(&self, line: &str, pos: usize, _: &Context<'_>) -> Option<String> {
        if !line.starts_with('/') || pos < 2 || line.contains(' ') {
            return None;
        }
        self.commands
            .iter()
            .find(|(name, _)| name.starts_with(line) && *name != line)
            .map(|(name, _)| name[line.len()..].to_string())
    }
}

impl Highlighter for KaraHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(style(hint).dim().to_string())
    }
}

impl Validator for KaraHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        // A trailing backslash continues the input on the next line.
        if ctx.input().ends_with('\\') {
            Ok(ValidationResult::Incomplete)
        } else {
            Ok(ValidationResult::Valid(None))
        }
    }
}

impl Helper for KaraHelper {}

pub fn interactive(
    rt: &tokio::runtime::Runtime,
    opts: &Options,
    resume: bool,
) -> anyhow::Result<i32> {
    let renderer = Renderer::new(false, false);
    let mut session = build_session(
        rt,
        opts,
        resume,
        Consent::Ask,
        Arc::new(TerminalApprover),
        renderer,
    )?;
    banner(&session);

    let mut rl: Editor<KaraHelper, rustyline::history::FileHistory> = Editor::new()?;
    rl.set_helper(Some(KaraHelper {
        commands: commands::COMMANDS.to_vec(),
    }));
    let history_file = session.app.paths.history_file();
    let _ = rl.load_history(&history_file);

    loop {
        let prompt = if session.agent.pending_plan().is_some() {
            format!("{} ", style("plan›").cyan().bold())
        } else {
            format!("{} ", style("›").cyan().bold())
        };
        let line = match rl.readline(&prompt) {
            Ok(l) => l,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        let input = line.replace("\\\n", "\n");
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(input);
        let _ = rl.save_history(&history_file);

        if input.starts_with('/') {
            match commands::handle(&mut session, rt, input) {
                Ok(Action::Continue) => {}
                Ok(Action::Exit) => break,
                Err(e) => eprintln!("{} {e:#}", style("error:").red()),
            }
            continue;
        }
        if session.agent.pending_plan().is_some()
            && matches!(
                input.to_ascii_lowercase().as_str(),
                "y" | "yes" | "approve" | "go" | "ok" | "do it" | "proceed"
            )
        {
            session.approve(rt);
            continue;
        }
        if session.runtime.provider.is_none() {
            println!("{}", style(no_model_message()).yellow());
            continue;
        }
        session.turn(rt, input, AgentMode::Execute);
        println!();
    }
    rt.block_on(session.runtime.shutdown());
    Ok(0)
}

pub fn run_once(
    rt: &tokio::runtime::Runtime,
    opts: &Options,
    prompt: &str,
    mode: AgentMode,
    yes: bool,
    json: bool,
) -> anyhow::Result<i32> {
    if prompt.trim().is_empty() && mode != AgentMode::Review {
        anyhow::bail!("give the task as an argument, e.g. kara run \"fix the failing tests\"");
    }
    let approver: Arc<dyn Approver> = if yes {
        Arc::new(ApproveOrdinary)
    } else {
        Arc::new(DenyAll)
    };
    let renderer = Renderer::new(false, json);
    let mut session = build_session(rt, opts, false, Consent::Never, approver, renderer)?;
    if session.runtime.provider.is_none() {
        anyhow::bail!("{}", no_model_message());
    }
    let task = if prompt.trim().is_empty() {
        "Review my current diff."
    } else {
        prompt
    };
    let r = session.turn(rt, task, mode);
    rt.block_on(session.runtime.shutdown());
    Ok(match r.outcome {
        kara_protocol::TurnOutcome::Completed | kara_protocol::TurnOutcome::AwaitingApproval => 0,
        _ => 1,
    })
}

pub fn undo_cli(opts: &Options) -> anyhow::Result<i32> {
    let app = App::load(opts)?;
    let Some(id) = app.sessions.latest_for(&app.root)? else {
        println!("No Kara session for this repository.");
        return Ok(1);
    };
    let ctx = app.tool_context(&id)?;
    let mut j = ctx.journal();
    match j.last_live_batch() {
        None => {
            println!("Nothing to undo.");
            return Ok(1);
        }
        Some(b) => println!("Undoing: {} ({} files)", b.label, b.entries.len()),
    }
    let report = j.undo(None)?;
    commands::print_undo(&report);
    Ok(if report.conflicts.is_empty() && report.errors.is_empty() {
        0
    } else {
        1
    })
}

pub fn sessions_cli(opts: &Options) -> anyhow::Result<i32> {
    let app = App::load(opts)?;
    let list = app.sessions.list(Some(&app.root), 20)?;
    if list.is_empty() {
        println!("No sessions for {}", app.root.display());
    }
    for s in list {
        println!(
            "{}  {}  {:>3} turns  {}",
            &s.id[..8],
            s.updated.get(..16).unwrap_or(&s.updated),
            s.turns,
            if s.title.is_empty() {
                "(empty)"
            } else {
                &s.title
            }
        );
    }
    Ok(0)
}
