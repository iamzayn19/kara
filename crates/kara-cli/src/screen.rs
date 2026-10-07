//! Full-screen interactive mode: a Claude-Code-style terminal UI built on
//! ratatui + crossterm, in place of the old scrolling rustyline REPL.
//!
//! Scope for this first pass: the chat loop (type, stream, see tool calls
//! and diffs render live) and permission prompts get the full treatment.
//! Slash commands (`/model`, `/doctor`, `/help`, ...) still render through
//! the original plain-terminal code in `commands.rs`; we drop out of the
//! alternate screen for the duration of one command and come back, rather
//! than reimplement every command's output as styled lines. Known
//! follow-ups: no scrollback (Page Up/Down), no multi-line paste editing.

use crate::app::Options;
use crate::commands::{self, Action};
use crate::render::Renderer;
use crate::theme;
use crate::transcript::Transcript;
use crate::tui::{self, Session, TerminalApprover};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use kara_agent::approver::Approver;
use kara_agent::TurnResult;
use kara_inference::source::Consent;
use kara_protocol::{AgentMode, PermissionDecision, PermissionRequest};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Puts the terminal into raw + alternate-screen mode and guarantees it is
/// restored on drop, including on panic.
struct RawScreen;

impl RawScreen {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(RawScreen)
    }
}

impl Drop for RawScreen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Shared handle the agent-event sink uses to repaint mid-turn, from
/// whichever thread is driving the turn.
#[derive(Clone)]
struct Ui {
    term: Arc<Mutex<Term>>,
    transcript: Transcript,
    banner: Arc<Vec<Line<'static>>>,
    status: Arc<Mutex<String>>,
    /// Lines held back from the bottom by Page Up/Down. 0 = pinned to the
    /// live tail (the normal, auto-scrolling state).
    scroll: Arc<Mutex<usize>>,
}

impl Ui {
    /// Move the scrollback window; positive scrolls up (back in history).
    fn scroll_by(&self, delta: isize) {
        let total = self.transcript.lines().len();
        let mut s = self.scroll.lock().unwrap();
        *s = (*s as isize + delta).clamp(0, total as isize) as usize;
    }

    fn scroll_to_bottom(&self) {
        *self.scroll.lock().unwrap() = 0;
    }

    fn draw(&self, bottom: Line<'static>, bottom_title: &str) {
        let mut term = self.term.lock().unwrap();
        let banner = self.banner.clone();
        let lines = self.transcript.lines();
        let status_text = self.status.lock().unwrap().clone();
        let scroll = *self.scroll.lock().unwrap();
        let _ = term.draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(banner.len() as u16),
                    Constraint::Min(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .split(area);

            f.render_widget(Paragraph::new((*banner).clone()), chunks[0]);

            let visible = chunks[1].height as usize;
            let total = lines.len();
            // `scroll` lines are held back from the bottom; 0 means pinned
            // to the live tail.
            let end = total.saturating_sub(scroll.min(total));
            let start = end.saturating_sub(visible.max(1));
            let tail: Vec<Line<'static>> = lines[start..end].to_vec();
            f.render_widget(Paragraph::new(tail).wrap(Wrap { trim: false }), chunks[1]);
            let status = if scroll > 0 {
                format!("{status_text} · scrolled back {scroll} lines (End to jump to latest)")
            } else {
                status_text
            };

            let input_block = Block::default()
                .borders(Borders::ALL)
                .border_style(theme::accent_dim())
                .title(bottom_title);
            f.render_widget(
                Paragraph::new(bottom)
                    .block(input_block)
                    .wrap(Wrap { trim: true }),
                chunks[2],
            );

            f.render_widget(
                Paragraph::new(Line::styled(status, theme::dim())),
                chunks[3],
            );
        });
    }
}

/// Permission prompts drop out of the alternate screen and use the plain
/// terminal prompt (readable, no risk of fighting the ratatui buffer), then
/// hand the screen back.
struct FullscreenApprover {
    ui: Ui,
}

#[async_trait::async_trait]
impl Approver for FullscreenApprover {
    async fn decide(&self, r: &PermissionRequest) -> PermissionDecision {
        let r = r.clone();
        let ui = self.ui.clone();
        tokio::task::spawn_blocking(move || {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            let _ = disable_raw_mode();
            let decision = tui::prompt_permission(&r);
            let _ = enable_raw_mode();
            let _ = execute!(io::stdout(), EnterAlternateScreen);
            ui.draw(Line::raw(""), "›");
            decision
        })
        .await
        .unwrap_or(PermissionDecision::Deny)
    }
}

fn banner_lines(session: &Session) -> Vec<Line<'static>> {
    use ratatui::text::Span;
    let app = &session.app;
    let model = if session.inference.provider.is_some() {
        session.model_label()
    } else {
        "none yet — /model auto to pick one".to_string()
    };
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
    vec![
        Line::from(vec![
            Span::styled("Kara", theme::accent()),
            Span::raw(format!(
                " {} · your code, your machine, your AI",
                kara_core::VERSION
            )),
        ]),
        Line::from(vec![
            Span::styled("model   ", theme::dim()),
            Span::raw(model),
        ]),
        Line::from(vec![
            Span::styled("repo    ", theme::dim()),
            Span::raw(format!(
                "{} ({git}){}",
                app.repo_name(),
                if langs.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", langs.join(", "))
                }
            )),
        ]),
        Line::from(vec![
            Span::styled("perms   ", theme::dim()),
            Span::raw(format!(
                "{} · /help for commands · Ctrl-C cancels · Ctrl-D exits",
                app.config.permissions.mode.as_str()
            )),
        ]),
        Line::raw(""),
    ]
}

/// Single-line input editor: enough for the common case, not a rustyline
/// replacement (no multi-line continuation, no fuzzy completion yet).
#[derive(Default)]
struct Input {
    text: String,
    cursor: usize,
    history: Vec<String>,
    hist_idx: Option<usize>,
}

impl Input {
    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    fn set(&mut self, s: String) {
        self.cursor = s.chars().count();
        self.text = s;
    }

    fn insert(&mut self, c: char) {
        let mut chars = self.chars();
        chars.insert(self.cursor, c);
        self.text = chars.into_iter().collect();
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut chars = self.chars();
        chars.remove(self.cursor - 1);
        self.text = chars.into_iter().collect();
        self.cursor -= 1;
    }

    fn delete(&mut self) {
        let mut chars = self.chars();
        if self.cursor < chars.len() {
            chars.remove(self.cursor);
            self.text = chars.into_iter().collect();
        }
    }

    fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars().len());
    }

    /// Single visual line for the input box. A continued (backslash) line
    /// holds real `\n`s; they're shown as `⏎` here since the box is one
    /// row tall, but submitted verbatim.
    fn line(&self) -> Line<'static> {
        let display = |c: char| if c == '\n' { '⏎' } else { c };
        let chars = self.chars();
        let (before, after) = chars.split_at(self.cursor.min(chars.len()));
        let cursor_char = after.first().copied();
        let after_rest: String = after.iter().skip(1).map(|&c| display(c)).collect();
        use ratatui::text::Span;
        let mut spans = vec![
            Span::styled("› ", theme::user()),
            Span::raw(before.iter().map(|&c| display(c)).collect::<String>()),
        ];
        spans.push(Span::styled(
            cursor_char
                .map(display)
                .map(String::from)
                .unwrap_or_else(|| " ".into()),
            ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        ));
        spans.push(Span::raw(after_rest));
        Line::from(spans)
    }
}

/// Cancellable turn: runs the agent on a scoped OS thread while this thread
/// polls crossterm for Ctrl-C, so the keyboard cancels even though raw mode
/// stops the usual SIGINT delivery.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// "Kara is working… ⠹ 3s (Ctrl-C to stop)" — ticks even when the agent is
/// quiet (e.g. mid tool-call), so a slow step never looks like a freeze.
fn working_line(started: std::time::Instant, frame: usize) -> Line<'static> {
    Line::styled(
        format!(
            "Kara is working… {} {}s (Ctrl-C to stop)",
            SPINNER[frame % SPINNER.len()],
            started.elapsed().as_secs()
        ),
        theme::dim(),
    )
}

fn run_turn(
    session: &mut Session,
    rt: &tokio::runtime::Runtime,
    task: &str,
    mode: AgentMode,
    ui: &Ui,
) -> TurnResult {
    let started = chrono::Utc::now().to_rfc3339();
    let token = CancellationToken::new();
    session.agent.ctx.cancel = token.clone();
    let result = std::thread::scope(|s| {
        let agent = &mut session.agent;
        let handle = s.spawn(move || rt.block_on(agent.run_turn(task, mode)));
        let spin_start = std::time::Instant::now();
        let mut frame = 0usize;
        while !handle.is_finished() {
            if matches!(event::poll(Duration::from_millis(80)), Ok(true)) {
                if let Ok(Event::Key(k)) = event::read() {
                    if is_ctrl_c(&k) {
                        token.cancel();
                    }
                }
            }
            frame += 1;
            ui.draw(working_line(spin_start, frame), "working");
        }
        handle.join().unwrap_or(TurnResult {
            turn_id: 0,
            mode,
            outcome: kara_protocol::TurnOutcome::Error,
            summary: "the turn thread panicked".into(),
            changed_files: vec![],
            stats: Default::default(),
            last_test: None,
        })
    });
    session.record(&result, task, &started);
    result
}

fn is_ctrl_c(k: &KeyEvent) -> bool {
    k.kind == KeyEventKind::Press
        && k.code == KeyCode::Char('c')
        && k.modifiers.contains(KeyModifiers::CONTROL)
}

/// Carry out a pending plan, same cancellable-with-spinner treatment as
/// `run_turn` instead of the plain blocking `Session::approve`.
fn run_approve(session: &mut Session, rt: &tokio::runtime::Runtime, ui: &Ui) {
    let Some((task, _)) = session.agent.pending_plan().cloned() else {
        return;
    };
    let started = chrono::Utc::now().to_rfc3339();
    let token = CancellationToken::new();
    session.agent.ctx.cancel = token.clone();
    let result = std::thread::scope(|s| {
        let agent = &mut session.agent;
        let handle = s.spawn(move || rt.block_on(agent.approve_plan()));
        let spin_start = std::time::Instant::now();
        let mut frame = 0usize;
        while !handle.is_finished() {
            if matches!(event::poll(Duration::from_millis(80)), Ok(true)) {
                if let Ok(Event::Key(k)) = event::read() {
                    if is_ctrl_c(&k) {
                        token.cancel();
                    }
                }
            }
            frame += 1;
            ui.draw(working_line(spin_start, frame), "working");
        }
        handle.join().unwrap_or(None)
    });
    if let Some(r) = &result {
        session.record(r, &format!("(approved plan) {task}"), &started);
    }
}

pub fn interactive(
    rt: &tokio::runtime::Runtime,
    opts: &Options,
    resume: bool,
) -> anyhow::Result<i32> {
    let transcript = Transcript::default();
    let t2 = transcript.clone();
    // The real-time sink: redraw is wired up once the Ui exists below, via
    // `ui_cell`; until then events just land in the buffer.
    let ui_cell: Arc<Mutex<Option<Ui>>> = Arc::new(Mutex::new(None));
    let uc = ui_cell.clone();
    let sink = Arc::new(move |e: kara_protocol::AgentEvent| {
        t2.handle(&e);
        if let Some(ui) = uc.lock().unwrap().as_ref() {
            ui.draw(
                Line::styled("Kara is working… (Ctrl-C to stop)", theme::dim()),
                "working",
            );
        }
    });

    let mut session = tui::build_session_with_sink(
        rt,
        opts,
        resume,
        Consent::Ask,
        Arc::new(TerminalApprover), // replaced below once the Ui exists
        Renderer::new(false, false),
        sink,
    )?;

    let _raw = RawScreen::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let term = Arc::new(Mutex::new(Terminal::new(backend)?));
    let banner = Arc::new(banner_lines(&session));
    let status = Arc::new(Mutex::new(format!(
        "kara {} · {}",
        kara_core::VERSION,
        session.app.repo_name()
    )));
    let ui = Ui {
        term,
        transcript: transcript.clone(),
        banner,
        status,
        scroll: Arc::new(Mutex::new(0)),
    };
    *ui_cell.lock().unwrap() = Some(ui.clone());

    // Swap in the approver that knows how to leave/re-enter the alt screen.
    let approver: Arc<dyn Approver> = Arc::new(FullscreenApprover { ui: ui.clone() });
    session.agent.set_approver(approver);

    if !session.inference.is_available() {
        let g = session
            .inference
            .guidance
            .clone()
            .unwrap_or_else(tui::no_model_message);
        transcript.push(Line::styled(g, theme::warn()));
    }

    let mut input = Input::default();
    ui.draw(input.line(), "›");

    loop {
        let ev = match event::read() {
            Ok(e) => e,
            Err(_) => continue,
        };
        let Event::Key(k) = ev else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        match k.code {
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if input.text.is_empty() {
                    break;
                }
                // Non-empty line: ignored, same as a shell's Ctrl-D.
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                input.set(String::new());
            }
            KeyCode::Char(c) => input.insert(c),
            KeyCode::Backspace => input.backspace(),
            KeyCode::Delete => input.delete(),
            KeyCode::Left => input.left(),
            KeyCode::Right => input.right(),
            KeyCode::Home => input.cursor = 0,
            KeyCode::End => input.cursor = input.chars().len(),
            KeyCode::PageUp => {
                ui.scroll_by(10);
                ui.draw(input.line(), "›");
                continue;
            }
            KeyCode::PageDown => {
                ui.scroll_by(-10);
                ui.draw(input.line(), "›");
                continue;
            }
            KeyCode::Up => {
                if !input.history.is_empty() {
                    let idx = input
                        .hist_idx
                        .map(|i| i.saturating_sub(1))
                        .unwrap_or(input.history.len() - 1);
                    input.hist_idx = Some(idx);
                    input.set(input.history[idx].clone());
                }
            }
            KeyCode::Down => {
                if let Some(i) = input.hist_idx {
                    if i + 1 < input.history.len() {
                        input.hist_idx = Some(i + 1);
                        input.set(input.history[i + 1].clone());
                    } else {
                        input.hist_idx = None;
                        input.set(String::new());
                    }
                }
            }
            KeyCode::Enter => {
                // A trailing backslash continues input on a new line
                // instead of submitting, same as the plain rustyline REPL.
                if input.text.ends_with('\\') {
                    let mut t = input.text.clone();
                    t.pop();
                    t.push('\n');
                    input.set(t);
                    ui.draw(input.line(), "›");
                    continue;
                }
                let line = input.text.trim().to_string();
                input.set(String::new());
                input.hist_idx = None;
                if line.is_empty() {
                    ui.draw(input.line(), "›");
                    continue;
                }
                input.history.push(line.clone());

                if line.starts_with('/') {
                    ui.scroll_to_bottom();
                    transcript.push_user(&line);
                    let keep_going =
                        drop_to_plain(&ui, || match commands::handle(&mut session, rt, &line) {
                            Ok(Action::Continue) => true,
                            Ok(Action::Exit) => false,
                            Err(e) => {
                                eprintln!("error: {e:#}");
                                true
                            }
                        });
                    if !keep_going {
                        break;
                    }
                    ui.draw(input.line(), "›");
                    continue;
                }
                if session.agent.pending_plan().is_some()
                    && matches!(
                        line.to_ascii_lowercase().as_str(),
                        "y" | "yes" | "approve" | "go" | "ok" | "do it" | "proceed"
                    )
                {
                    ui.scroll_to_bottom();
                    transcript.push_user(&line);
                    run_approve(&mut session, rt, &ui);
                    ui.draw(input.line(), "›");
                    continue;
                }
                if session.inference.provider.is_none() {
                    transcript.push(Line::styled(tui::no_model_message(), theme::warn()));
                    ui.draw(input.line(), "›");
                    continue;
                }
                ui.scroll_to_bottom();
                transcript.push_user(&line);
                let prompt = if session.agent.pending_plan().is_some() {
                    "plan"
                } else {
                    "working"
                };
                ui.draw(
                    Line::styled("Kara is working… (Ctrl-C to stop)", theme::dim()),
                    prompt,
                );
                run_turn(&mut session, rt, &line, AgentMode::Execute, &ui);
                ui.draw(input.line(), "›");
                continue;
            }
            _ => continue,
        }
        let title = if session.agent.pending_plan().is_some() {
            "plan"
        } else {
            "›"
        };
        ui.draw(input.line(), title);
    }

    drop(ui_cell);
    rt.block_on(session.inference.shutdown());
    Ok(0)
}

/// Leave the alt screen for the duration of `f` (slash commands render with
/// the plain `commands.rs`/`println!` path), then come back.
fn drop_to_plain(ui: &Ui, f: impl FnOnce() -> bool) -> bool {
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
    println!(
        "{}",
        crate::theme::console_accent().apply_to("── Kara ──────────────────────────────────────")
    );
    let keep_going = f();
    if keep_going {
        println!(
            "{}",
            crate::theme::console_accent()
                .apply_to("── press Enter to return ────────────────────────")
        );
        let mut discard = String::new();
        let _ = std::io::stdin().read_line(&mut discard);
    }
    let _ = enable_raw_mode();
    let _ = execute!(io::stdout(), EnterAlternateScreen);
    let _ = ui.term.lock().unwrap().clear();
    keep_going
}
