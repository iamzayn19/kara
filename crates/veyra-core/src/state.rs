//! Structured task state.
//!
//! The agent does not rely on the model remembering raw chat history. This
//! state is updated by the agent loop (and by the model through the
//! `update_plan` tool), rendered into a compact "working memory" block on every
//! model call, and persisted so sessions can be resumed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use veyra_protocol::{PlanStep, StepStatus, TestReport};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TaskState {
    pub task: String,
    pub plan: Vec<PlanStep>,
    pub hypothesis: Option<String>,
    pub completion_criteria: Vec<String>,
    /// Short facts learned while working ("auth uses SessionStore in lib/session.rb").
    pub observations: Vec<String>,
    /// One line per tool call ("grep 'authenticate' -> 12 matches").
    pub actions: Vec<String>,
    pub files_read: BTreeSet<String>,
    pub files_changed: BTreeSet<String>,
    pub tests: Vec<TestReport>,
    pub failures: Vec<String>,
    /// Number of recovery attempts after failing verification in this task.
    pub retries: u32,
}

const MAX_ACTIONS_SHOWN: usize = 12;
const MAX_OBSERVATIONS_SHOWN: usize = 10;

impl TaskState {
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            ..Default::default()
        }
    }

    pub fn record_action(&mut self, line: impl Into<String>) {
        self.actions.push(line.into());
    }

    pub fn observe(&mut self, fact: impl Into<String>) {
        let fact = fact.into();
        if !self.observations.contains(&fact) {
            self.observations.push(fact);
        }
    }

    pub fn last_test(&self) -> Option<&TestReport> {
        self.tests.last()
    }

    /// True when files changed after the most recent test run (or no test ran).
    pub fn edits_unverified(&self, edits_since_test: bool) -> bool {
        !self.files_changed.is_empty() && (self.tests.is_empty() || edits_since_test)
    }

    /// Compact rendering for the model's context window.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("## Working memory (maintained by Veyra)\n");
        out.push_str(&format!("Task: {}\n", self.task));
        if !self.plan.is_empty() {
            out.push_str("Plan:\n");
            for (i, step) in self.plan.iter().enumerate() {
                let mark = match step.status {
                    StepStatus::Done => "x",
                    StepStatus::InProgress => ">",
                    StepStatus::Skipped => "-",
                    StepStatus::Pending => " ",
                };
                out.push_str(&format!("  [{mark}] {}. {}\n", i + 1, step.title));
            }
        }
        if let Some(h) = &self.hypothesis {
            out.push_str(&format!("Current hypothesis: {h}\n"));
        }
        if !self.completion_criteria.is_empty() {
            out.push_str(&format!(
                "Done when: {}\n",
                self.completion_criteria.join("; ")
            ));
        }
        if !self.observations.is_empty() {
            out.push_str("Observations:\n");
            let skip = self
                .observations
                .len()
                .saturating_sub(MAX_OBSERVATIONS_SHOWN);
            for o in &self.observations[skip..] {
                out.push_str(&format!("  - {o}\n"));
            }
        }
        if !self.files_read.is_empty() {
            out.push_str(&format!(
                "Files read: {}\n",
                join_limited(self.files_read.iter(), 20)
            ));
        }
        if !self.files_changed.is_empty() {
            out.push_str(&format!(
                "Files changed by Veyra: {}\n",
                join_limited(self.files_changed.iter(), 30)
            ));
        }
        if let Some(t) = self.last_test() {
            out.push_str(&format!(
                "Last test run: `{}` -> {} (exit {:?}){}\n",
                t.command,
                if t.succeeded() { "PASS" } else { "FAIL" },
                t.exit_code,
                if t.failed_tests.is_empty() {
                    String::new()
                } else {
                    format!("; failing: {}", t.failed_tests.join(", "))
                }
            ));
        }
        if self.retries > 0 {
            out.push_str(&format!("Recovery attempts so far: {}\n", self.retries));
        }
        if !self.actions.is_empty() {
            out.push_str("Recent actions:\n");
            let skip = self.actions.len().saturating_sub(MAX_ACTIONS_SHOWN);
            for a in &self.actions[skip..] {
                out.push_str(&format!("  - {a}\n"));
            }
        }
        out
    }
}

fn join_limited<'a>(items: impl Iterator<Item = &'a String>, max: usize) -> String {
    let all: Vec<&String> = items.collect();
    let mut s = all
        .iter()
        .take(max)
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if all.len() > max {
        s.push_str(&format!(" (+{} more)", all.len() - max));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_is_compact_and_bounded() {
        let mut s = TaskState::new("fix auth");
        for i in 0..100 {
            s.record_action(format!("action {i}"));
        }
        s.plan.push(PlanStep {
            title: "reproduce".into(),
            status: StepStatus::Done,
        });
        s.hypothesis = Some("token expiry off by one".into());
        let r = s.render();
        assert!(r.contains("[x] 1. reproduce"));
        assert!(r.contains("action 99"));
        assert!(!r.contains("action 10\n"));
        assert!(r.contains("token expiry"));
    }

    #[test]
    fn unverified_edits() {
        let mut s = TaskState::new("t");
        assert!(!s.edits_unverified(false));
        s.files_changed.insert("a.rs".into());
        assert!(s.edits_unverified(false));
        s.tests.push(TestReport {
            command: "cargo test".into(),
            exit_code: Some(0),
            passed: Some(1),
            failed: Some(0),
            duration_ms: 1,
            failed_tests: vec![],
        });
        assert!(!s.edits_unverified(false));
        assert!(s.edits_unverified(true));
    }
}
