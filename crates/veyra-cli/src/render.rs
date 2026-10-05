//! Terminal rendering of agent events.

use console::style;
use std::io::Write;
use std::sync::{Arc, Mutex};
use veyra_protocol::{AgentEvent, ChangeKind, NoticeLevel, Phase, StepStatus, TurnOutcome};

#[derive(Default)]
struct State {
    mid_line: bool,
    tool_open: bool,
    thinking_shown: bool,
    /// The transient "thinking…" line is currently on screen.
    thinking_line: bool,
    /// Suppress raw `<tool_call>` text some models stream as content.
    suppress: bool,
    pending: String,
    last_phase: Option<Phase>,
}

#[derive(Clone)]
pub struct Renderer {
    state: Arc<Mutex<State>>,
    pub show_reasoning: bool,
    pub json: bool,
    tty: bool,
}

impl Renderer {
    pub fn new(show_reasoning: bool, json: bool) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            show_reasoning,
            json,
            tty: console::Term::stdout().is_term(),
        }
    }

    /// Remove the transient "thinking…" indicator if it is showing.
    fn clear_thinking(&self, st: &mut State) {
        if st.thinking_line {
            self.out("\r\x1b[2K");
            st.thinking_line = false;
        }
    }

    fn out(&self, s: &str) {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(s.as_bytes());
        let _ = o.flush();
    }

    fn newline_if_mid(&self, st: &mut State) {
        self.clear_thinking(st);
        if st.mid_line || st.tool_open {
            self.out("\n");
            st.mid_line = false;
            st.tool_open = false;
        }
    }

    pub fn handle(&self, e: &AgentEvent) {
        if self.json {
            self.out(&format!(
                "{}\n",
                serde_json::to_string(e).unwrap_or_default()
            ));
            return;
        }
        let mut st = self.state.lock().unwrap();
        match e {
            AgentEvent::TurnStarted { .. } => {
                *st = State::default();
            }
            AgentEvent::Phase { phase } => {
                if *phase == Phase::Recover && st.last_phase != Some(Phase::Recover) {
                    self.newline_if_mid(&mut st);
                    self.out(&format!(
                        "{}\n",
                        style("→ tests failed; investigating and retrying").yellow()
                    ));
                }
                st.last_phase = Some(*phase);
            }
            AgentEvent::ReasoningDelta { text } => {
                if self.show_reasoning {
                    if st.tool_open {
                        self.newline_if_mid(&mut st);
                    }
                    self.out(&style(text).dim().to_string());
                    st.mid_line = !text.ends_with('\n');
                } else if !st.thinking_shown && self.tty {
                    st.thinking_shown = true;
                    self.newline_if_mid(&mut st);
                    self.out(&style("· thinking…").dim().to_string());
                    st.thinking_line = true;
                }
            }
            AgentEvent::AssistantDelta { text } => {
                if st.tool_open {
                    self.newline_if_mid(&mut st);
                }
                if st.suppress {
                    return;
                }
                st.pending.push_str(text);
                if let Some(i) = st.pending.find("<tool_call>") {
                    let before = st.pending[..i].to_string();
                    self.out(&before);
                    st.pending.clear();
                    st.suppress = true;
                    return;
                }
                // Hold back a possible partial "<tool_call>" prefix.
                let keep = [
                    "<",
                    "<t",
                    "<to",
                    "<too",
                    "<tool",
                    "<tool_",
                    "<tool_c",
                    "<tool_ca",
                    "<tool_cal",
                    "<tool_call",
                ]
                .iter()
                .filter(|p| st.pending.ends_with(*p))
                .map(|p| p.len())
                .max()
                .unwrap_or(0);
                let emit_len = st.pending.len() - keep;
                let emit = st.pending[..emit_len].to_string();
                st.pending.drain(..emit_len);
                if !emit.is_empty() {
                    self.clear_thinking(&mut st);
                    self.out(&emit);
                    st.mid_line = !emit.ends_with('\n');
                }
            }
            AgentEvent::AssistantMessage { .. } => {
                if !st.pending.is_empty() && !st.suppress {
                    let p = std::mem::take(&mut st.pending);
                    self.out(&p);
                }
                self.newline_if_mid(&mut st);
            }
            AgentEvent::ToolStarted { summary, .. } => {
                st.pending.clear();
                st.suppress = false;
                st.thinking_shown = false;
                self.newline_if_mid(&mut st);
                self.out(&format!("{} {}", style("→").cyan(), summary));
                st.tool_open = true;
            }
            AgentEvent::ToolFinished {
                ok,
                summary,
                duration_ms,
                ..
            } => {
                let mark = if *ok {
                    style("✓").green()
                } else {
                    style("✗").red()
                };
                let time = if *duration_ms >= 1000 {
                    format!(" ({:.1}s)", *duration_ms as f64 / 1000.0)
                } else {
                    String::new()
                };
                if st.tool_open {
                    self.out(&format!(
                        "  {mark} {}{}\n",
                        style(summary).dim(),
                        style(time).dim()
                    ));
                } else {
                    self.out(&format!("  {mark} {}\n", style(summary).dim()));
                }
                st.tool_open = false;
                st.mid_line = false;
            }
            AgentEvent::FileChanged { change } => {
                self.newline_if_mid(&mut st);
                let (add, del) = diff_stat(&change.diff);
                let sym = match change.kind {
                    ChangeKind::Created => style("+").green(),
                    ChangeKind::Deleted => style("-").red(),
                    _ => style("~").yellow(),
                };
                self.out(&format!(
                    "  {sym} {} {}\n",
                    change.path,
                    style(format!("(+{add} −{del})")).dim()
                ));
                if add + del <= 24 && change.kind != ChangeKind::Deleted {
                    self.out(&indent(
                        &colorize_diff(&strip_headers(&change.diff)),
                        "      ",
                    ));
                }
            }
            AgentEvent::TestFinished { report } => {
                self.newline_if_mid(&mut st);
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
                    self.out(&format!(
                        "  {} {}\n",
                        style("tests passed:").green().bold(),
                        counts
                    ));
                } else {
                    self.out(&format!(
                        "  {} {}\n",
                        style("tests failed:").red().bold(),
                        counts
                    ));
                    for t in report.failed_tests.iter().take(8) {
                        self.out(&format!("    {} {}\n", style("✗").red(), t));
                    }
                }
            }
            AgentEvent::PlanUpdated { steps, hypothesis } => {
                self.newline_if_mid(&mut st);
                for s in steps {
                    let mark = match s.status {
                        StepStatus::Done => style("[x]").green(),
                        StepStatus::InProgress => style("[>]").cyan(),
                        StepStatus::Skipped => style("[-]").dim(),
                        StepStatus::Pending => style("[ ]").dim(),
                    };
                    self.out(&format!("  {mark} {}\n", s.title));
                }
                if let Some(h) = hypothesis {
                    self.out(&format!("  {} {}\n", style("hypothesis:").dim(), h));
                }
            }
            AgentEvent::Notice { level, message } => {
                self.newline_if_mid(&mut st);
                let s = match level {
                    NoticeLevel::Info => style(format!("· {message}")).dim(),
                    NoticeLevel::Warning => style(format!("! {message}")).yellow(),
                    NoticeLevel::Error => style(format!("✗ {message}")).red(),
                };
                self.out(&format!("{s}\n"));
            }
            AgentEvent::TurnFinished {
                outcome,
                changed_files,
                usage,
                ..
            } => {
                if !st.pending.is_empty() && !st.suppress {
                    let p = std::mem::take(&mut st.pending);
                    self.out(&p);
                }
                self.newline_if_mid(&mut st);
                if !changed_files.is_empty() {
                    self.out(&format!("\n{}\n", style("Changed:").bold()));
                    for f in changed_files {
                        self.out(&format!("  {f}\n"));
                    }
                }
                match outcome {
                    TurnOutcome::Completed => {}
                    TurnOutcome::AwaitingApproval => self.out(&format!(
                        "\n{}\n",
                        style("Plan ready. Type /approve to carry it out, or describe changes to the plan.").cyan()
                    )),
                    TurnOutcome::Cancelled => self.out(&format!("{}\n", style("cancelled").yellow())),
                    TurnOutcome::Stalled => self.out(&format!("{}\n", style("stopped without finishing").yellow())),
                    TurnOutcome::Error => self.out(&format!("{}\n", style("stopped on an error").red())),
                }
                if usage.prompt_tokens + usage.completion_tokens > 0 {
                    self.out(&format!(
                        "{}\n",
                        style(format!(
                            "{} tokens in, {} out",
                            usage.prompt_tokens, usage.completion_tokens
                        ))
                        .dim()
                    ));
                }
            }
        }
    }
}

pub fn diff_stat(diff: &str) -> (usize, usize) {
    let mut a = 0;
    let mut d = 0;
    for l in diff.lines() {
        if l.starts_with('+') && !l.starts_with("+++") {
            a += 1;
        } else if l.starts_with('-') && !l.starts_with("---") {
            d += 1;
        }
    }
    (a, d)
}

fn strip_headers(diff: &str) -> String {
    diff.lines()
        .filter(|l| !l.starts_with("---") && !l.starts_with("+++"))
        .map(|l| format!("{l}\n"))
        .collect()
}

pub fn colorize_diff(diff: &str) -> String {
    let mut out = String::new();
    for l in diff.lines() {
        let s = if l.starts_with("+++") || l.starts_with("---") {
            style(l).bold().to_string()
        } else if l.starts_with('+') {
            style(l).green().to_string()
        } else if l.starts_with('-') {
            style(l).red().to_string()
        } else if l.starts_with("@@") {
            style(l).cyan().to_string()
        } else {
            style(l).dim().to_string()
        };
        out.push_str(&s);
        out.push('\n');
    }
    out
}

fn indent(s: &str, pad: &str) -> String {
    s.lines().map(|l| format!("{pad}{l}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats() {
        assert_eq!(diff_stat("--- a/x\n+++ b/x\n@@\n-a\n+b\n+c\n"), (2, 1));
    }
}
