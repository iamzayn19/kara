//! Turns `AgentEvent`s into styled lines for the full-screen TUI.
//!
//! This is the ratatui sibling of `render.rs` (which prints ANSI directly
//! for the plain scrolling/JSON output used by `kara run` and `--json`).
//! Same events, same meaning, different target: lines pushed into a shared
//! buffer the UI redraws from, instead of bytes written to stdout.

use crate::theme;
use kara_protocol::{AgentEvent, ChangeKind, NoticeLevel, StepStatus, TurnOutcome};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::sync::{Arc, Mutex};

/// Shared, append-only log the draw loop reads from. Cheap to clone.
#[derive(Clone, Default)]
pub struct Transcript {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    lines: Vec<Line<'static>>,
    /// Assistant text streamed so far this turn, not yet committed as a line.
    pending: String,
    tool_open: bool,
    thinking_shown: bool,
    suppress: bool,
}

fn plain(s: impl Into<String>) -> Line<'static> {
    Line::from(s.into())
}

fn styled(s: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(Span::styled(s.into(), style))
}

#[allow(dead_code)] // small API surface kept for follow-up work on the slash-command output
impl Transcript {
    pub fn lines(&self) -> Vec<Line<'static>> {
        self.inner.lock().unwrap().lines.clone()
    }

    pub fn push(&self, line: Line<'static>) {
        self.inner.lock().unwrap().lines.push(line);
    }

    pub fn push_plain(&self, s: impl Into<String>) {
        self.push(plain(s));
    }

    /// A line the user typed, echoed back into the transcript.
    pub fn push_user(&self, s: &str) {
        let mut inner = self.inner.lock().unwrap();
        for (i, l) in s.lines().enumerate() {
            let prefix = if i == 0 { "› " } else { "  " };
            inner.lines.push(Line::from(vec![
                Span::styled(prefix, theme::user()),
                Span::raw(l.to_string()),
            ]));
        }
        if s.is_empty() {
            inner.lines.push(styled("› ", theme::user()));
        }
    }

    pub fn push_system(&self, s: impl Into<String>) {
        self.push(styled(format!("· {}", s.into()), theme::dim()));
    }

    pub fn push_error(&self, s: impl Into<String>) {
        self.push(styled(format!("error: {}", s.into()), theme::err_bold()));
    }

    /// Flush any assistant text that never got a trailing newline.
    fn flush_pending(&self, inner: &mut Inner) {
        if !inner.pending.is_empty() && !inner.suppress {
            let p = std::mem::take(&mut inner.pending);
            for l in p.split('\n') {
                inner.lines.push(plain(l.to_string()));
            }
        }
        inner.pending.clear();
    }

    /// Feed one agent event in. Mirrors `render::Renderer::handle`.
    pub fn handle(&self, e: &AgentEvent) {
        let mut inner = self.inner.lock().unwrap();
        match e {
            AgentEvent::TurnStarted { .. } => {
                *inner = Inner::default();
            }
            AgentEvent::Phase { phase } => {
                if *phase == kara_protocol::Phase::Recover {
                    inner.lines.push(styled(
                        "→ tests failed; investigating and retrying",
                        theme::warn(),
                    ));
                }
            }
            AgentEvent::ReasoningDelta { .. } => {
                if !inner.thinking_shown {
                    inner.thinking_shown = true;
                    inner.lines.push(styled("· thinking…", theme::dim()));
                }
            }
            AgentEvent::AssistantDelta { text } => {
                if inner.suppress {
                    return;
                }
                inner.pending.push_str(text);
                if let Some(i) = inner.pending.find("<tool_call>") {
                    inner.pending.truncate(i);
                    inner.suppress = true;
                }
            }
            AgentEvent::AssistantMessage { .. } => {
                self.flush_pending(&mut inner);
            }
            AgentEvent::ToolStarted { summary, .. } => {
                inner.pending.clear();
                inner.suppress = false;
                inner.thinking_shown = false;
                inner.lines.push(Line::from(vec![
                    Span::styled("→ ", theme::accent_dim()),
                    Span::raw(summary.clone()),
                ]));
                inner.tool_open = true;
            }
            AgentEvent::ToolFinished {
                ok,
                summary,
                duration_ms,
                ..
            } => {
                let (mark, style) = if *ok {
                    ("✓", theme::ok())
                } else {
                    ("✗", theme::err())
                };
                let time = if *duration_ms >= 1000 {
                    format!(" ({:.1}s)", *duration_ms as f64 / 1000.0)
                } else {
                    String::new()
                };
                inner.lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(mark, style),
                    Span::raw(" "),
                    Span::styled(format!("{summary}{time}"), theme::dim()),
                ]));
                inner.tool_open = false;
            }
            AgentEvent::FileChanged { change } => {
                let (add, del) = crate::render::diff_stat(&change.diff);
                let (sym, style) = match change.kind {
                    ChangeKind::Created => ("+", theme::add()),
                    ChangeKind::Deleted => ("-", theme::del()),
                    _ => ("~", theme::warn()),
                };
                inner.lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(sym, style),
                    Span::raw(format!(" {} ", change.path)),
                    Span::styled(format!("(+{add} −{del})"), theme::dim()),
                ]));
                if add + del <= 24 && change.kind != ChangeKind::Deleted {
                    for l in change.diff.lines() {
                        if l.starts_with("---") || l.starts_with("+++") {
                            continue;
                        }
                        let style = if l.starts_with('+') {
                            theme::add()
                        } else if l.starts_with('-') {
                            theme::del()
                        } else if l.starts_with("@@") {
                            theme::hunk()
                        } else {
                            theme::dim()
                        };
                        inner.lines.push(styled(format!("      {l}"), style));
                    }
                }
            }
            AgentEvent::TestFinished { report } => {
                let counts = match (report.passed, report.failed) {
                    (Some(p), Some(f)) => format!("{p} passed, {f} failed"),
                    _ => format!(
                        "exit {}",
                        report
                            .exit_code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "?".into())
                    ),
                };
                if report.succeeded() {
                    inner.lines.push(Line::from(vec![
                        Span::styled("  tests passed: ", theme::ok_bold()),
                        Span::raw(counts),
                    ]));
                } else {
                    inner.lines.push(Line::from(vec![
                        Span::styled("  tests failed: ", theme::err_bold()),
                        Span::raw(counts),
                    ]));
                    for t in report.failed_tests.iter().take(8) {
                        inner.lines.push(styled(format!("    ✗ {t}"), theme::err()));
                    }
                }
            }
            AgentEvent::PlanUpdated { steps, hypothesis } => {
                for s in steps {
                    let (mark, style) = match s.status {
                        StepStatus::Done => ("[x]", theme::ok()),
                        StepStatus::InProgress => ("[>]", theme::accent_dim()),
                        StepStatus::Skipped => ("[-]", theme::dim()),
                        StepStatus::Pending => ("[ ]", theme::dim()),
                    };
                    inner.lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(mark, style),
                        Span::raw(format!(" {}", s.title)),
                    ]));
                }
                if let Some(h) = hypothesis {
                    inner.lines.push(Line::from(vec![
                        Span::styled("  hypothesis: ", theme::dim()),
                        Span::raw(h.clone()),
                    ]));
                }
            }
            AgentEvent::Notice { level, message } => {
                let style = match level {
                    NoticeLevel::Info => theme::dim(),
                    NoticeLevel::Warning => theme::warn(),
                    NoticeLevel::Error => theme::err(),
                };
                let mark = match level {
                    NoticeLevel::Info => "·",
                    NoticeLevel::Warning => "!",
                    NoticeLevel::Error => "✗",
                };
                inner.lines.push(styled(format!("{mark} {message}"), style));
            }
            AgentEvent::TurnFinished {
                outcome,
                changed_files,
                usage,
                ..
            } => {
                self.flush_pending(&mut inner);
                if !changed_files.is_empty() {
                    inner.lines.push(styled("Changed:", theme::bold()));
                    for f in changed_files {
                        inner.lines.push(plain(format!("  {f}")));
                    }
                }
                match outcome {
                    TurnOutcome::Completed => {}
                    TurnOutcome::AwaitingApproval => inner.lines.push(styled(
                        "Plan ready. Type /approve to carry it out, or describe changes to the plan.",
                        theme::accent_dim(),
                    )),
                    TurnOutcome::Cancelled => {
                        inner.lines.push(styled("cancelled", theme::warn()))
                    }
                    TurnOutcome::Stalled => inner
                        .lines
                        .push(styled("stopped without finishing", theme::warn())),
                    TurnOutcome::Error => {
                        inner.lines.push(styled("stopped on an error", theme::err()))
                    }
                }
                if usage.prompt_tokens + usage.completion_tokens > 0 {
                    inner.lines.push(styled(
                        format!(
                            "{} tokens in, {} out",
                            usage.prompt_tokens, usage.completion_tokens
                        ),
                        theme::dim(),
                    ));
                }
                inner.lines.push(plain(""));
            }
        }
    }
}
