//! Wire types shared between the Kara agent and its front ends.
//!
//! The terminal UI consumes [`AgentEvent`]s in-process. Editor clients (the
//! VS Code extension) receive the same events as JSON-RPC 2.0 notifications
//! over the stdio of `kara serve --stdio`. Keeping one event vocabulary means
//! the extension never needs its own copy of agent logic.

pub mod jsonrpc;

use serde::{Deserialize, Serialize};

/// Protocol version advertised by `initialize`. Bump the minor number for
/// additive changes and the major number for breaking ones.
pub const PROTOCOL_VERSION: &str = "1.0";

/// What the agent is doing for the current turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Normal engineering loop: understand, edit, test, review.
    #[default]
    Execute,
    /// Read-only planning. Produces a plan and waits for approval (`/morpheus`, `/plan`).
    Plan,
    /// Read-only risk review of the current diff (`/oracle`, `/review`).
    Review,
}

/// Coarse phase of the engineering loop, reported for UI status lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Understand,
    Search,
    Read,
    Plan,
    Edit,
    Test,
    Recover,
    Verify,
    Review,
    Summarize,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Understand => "understanding task",
            Phase::Search => "searching repository",
            Phase::Read => "reading code",
            Phase::Plan => "planning",
            Phase::Edit => "editing",
            Phase::Test => "running tests",
            Phase::Recover => "recovering from failure",
            Phase::Verify => "verifying",
            Phase::Review => "reviewing diff",
            Phase::Summarize => "summarizing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    InProgress,
    Done,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub title: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    /// Plan mode finished; the agent is waiting for the user to approve the plan.
    AwaitingApproval,
    Cancelled,
    /// The agent stopped because it could not make progress (repeated
    /// identical actions or exhausted recovery attempts). Not a usage limit.
    Stalled,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestReport {
    pub command: String,
    pub exit_code: Option<i32>,
    pub passed: Option<u32>,
    pub failed: Option<u32>,
    pub duration_ms: u64,
    pub failed_tests: Vec<String>,
}

impl TestReport {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
    /// Unified diff of this single change (may be truncated for huge files).
    pub diff: String,
    pub batch_id: u64,
}

/// Events emitted by the agent during a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TurnStarted {
        turn_id: u64,
        mode: AgentMode,
        task: String,
    },
    Phase {
        phase: Phase,
    },
    AssistantDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    AssistantMessage {
        text: String,
    },
    ToolStarted {
        call_id: String,
        tool: String,
        summary: String,
    },
    ToolFinished {
        call_id: String,
        tool: String,
        ok: bool,
        summary: String,
        duration_ms: u64,
    },
    FileChanged {
        change: FileChange,
    },
    TestFinished {
        report: TestReport,
    },
    PlanUpdated {
        steps: Vec<PlanStep>,
        hypothesis: Option<String>,
    },
    Notice {
        level: NoticeLevel,
        message: String,
    },
    TurnFinished {
        turn_id: u64,
        outcome: TurnOutcome,
        summary: String,
        changed_files: Vec<String>,
        usage: TokenUsage,
    },
}

/// Permission categories. Profiles map each category to allow / ask / deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Reading files inside the workspace.
    Read,
    /// Text or symbol search inside the workspace.
    Search,
    /// Read-only git operations (status, diff, log, show, blame).
    GitRead,
    /// Running the project's test command.
    Test,
    /// Running the project's lint, format-check or typecheck command.
    Lint,
    /// Running the project's build command.
    Build,
    /// Creating or modifying files inside the workspace.
    Write,
    /// Deleting files inside the workspace.
    Delete,
    /// Running an arbitrary shell command that is not obviously dangerous.
    Shell,
    /// Shell commands that may destroy data (rm -r, git reset --hard, ...).
    Destructive,
    /// Commands that reach the network (curl, package installs, ...).
    Network,
    GitCommit,
    GitPush,
    /// Any filesystem access outside the workspace.
    OutsideWorkspace,
    /// Credentials and secrets: SSH keys, cloud credentials, password stores, .env files.
    Secrets,
    /// Privilege escalation (sudo, doas, su, runas).
    Privileged,
}

impl ActionKind {
    /// High-risk boundaries are never granted for a whole session and are
    /// never silently allowed by any profile.
    pub fn is_hard_boundary(self) -> bool {
        matches!(
            self,
            ActionKind::GitPush
                | ActionKind::OutsideWorkspace
                | ActionKind::Secrets
                | ActionKind::Privileged
                | ActionKind::Destructive
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            ActionKind::Read => "read",
            ActionKind::Search => "search",
            ActionKind::GitRead => "git-read",
            ActionKind::Test => "tests",
            ActionKind::Lint => "lint",
            ActionKind::Build => "build",
            ActionKind::Write => "write",
            ActionKind::Delete => "delete",
            ActionKind::Shell => "shell",
            ActionKind::Destructive => "destructive",
            ActionKind::Network => "network",
            ActionKind::GitCommit => "git-commit",
            ActionKind::GitPush => "git-push",
            ActionKind::OutsideWorkspace => "outside-workspace",
            ActionKind::Secrets => "secrets",
            ActionKind::Privileged => "privileged",
        }
    }

    pub const ALL: [ActionKind; 16] = [
        ActionKind::Read,
        ActionKind::Search,
        ActionKind::GitRead,
        ActionKind::Test,
        ActionKind::Lint,
        ActionKind::Build,
        ActionKind::Write,
        ActionKind::Delete,
        ActionKind::Shell,
        ActionKind::Destructive,
        ActionKind::Network,
        ActionKind::GitCommit,
        ActionKind::GitPush,
        ActionKind::OutsideWorkspace,
        ActionKind::Secrets,
        ActionKind::Privileged,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: u64,
    pub tool: String,
    pub kinds: Vec<ActionKind>,
    /// One-line human description, e.g. `run: bundle exec rspec`.
    pub title: String,
    /// Extra detail: the full command, target path, or a diff preview.
    pub detail: String,
    pub reasons: Vec<String>,
    /// Whether "allow for this session" may be offered. False for hard boundaries.
    pub can_remember: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_type_tag() {
        let ev = AgentEvent::Phase { phase: Phase::Test };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(json, r#"{"type":"phase","phase":"test"}"#);
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn hard_boundaries_cover_high_risk_kinds() {
        for kind in [
            ActionKind::GitPush,
            ActionKind::Secrets,
            ActionKind::Privileged,
            ActionKind::OutsideWorkspace,
            ActionKind::Destructive,
        ] {
            assert!(kind.is_hard_boundary(), "{kind:?}");
        }
        assert!(!ActionKind::Read.is_hard_boundary());
        assert!(!ActionKind::Write.is_hard_boundary());
    }
}
