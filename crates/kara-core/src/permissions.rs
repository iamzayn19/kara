//! Permission modes and decisions.
//!
//! A tool invocation is classified (by `kara-sandbox`) into one or more
//! [`ActionKind`]s. The policy combines the decisions for each kind: any deny
//! wins, then any ask, otherwise allow.
//!
//! Modes:
//!
//! * `ask`: reading, searching and running tests are free; every edit and
//!   shell command asks.
//! * `workspace` (default): normal coding inside the workspace (edits, tests,
//!   lint, builds, git inspection) is free; anything risky or leaving the
//!   workspace asks.
//! * `full`: explicit opt-in to broad tool execution (shell, deletes, network,
//!   commits).
//!
//! Hard boundaries (secrets, privilege escalation, git push, destructive
//! commands, paths outside the workspace) are never `Allow` in any mode and
//! can never be remembered for the session: each occurrence needs explicit
//! consent. `full` can only come from user configuration or an explicit
//! interactive action, never from repository or workspace configuration.

use kara_protocol::ActionKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ask,
    #[default]
    Workspace,
    Full,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Ask, Mode::Workspace, Mode::Full];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Workspace => "workspace",
            Mode::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ask" => Some(Mode::Ask),
            "workspace" => Some(Mode::Workspace),
            "full" => Some(Mode::Full),
            _ => None,
        }
    }

    /// Higher is stricter.
    pub fn strictness(self) -> u8 {
        match self {
            Mode::Ask => 2,
            Mode::Workspace => 1,
            Mode::Full => 0,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Mode::Ask => "reads, search, git inspection and tests run freely; every edit and shell command asks",
            Mode::Workspace => "edits, tests, lint and builds inside the workspace run freely; risky or out-of-workspace actions ask",
            Mode::Full => "broad tool execution (shell, deletes, network, commits) runs freely; high-risk boundaries still ask",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Ask,
    Deny,
}

/// Static mode table.
pub fn mode_decision(mode: Mode, kind: ActionKind) -> PolicyDecision {
    use ActionKind::*;
    use PolicyDecision::*;
    match (mode, kind) {
        // Hard boundaries: never silently granted.
        (_, Secrets) | (_, Privileged) | (_, OutsideWorkspace) | (_, Destructive) => Ask,
        (Mode::Ask, GitPush) => Deny,
        (_, GitPush) => Ask,

        (_, Read) | (_, Search) | (_, GitRead) => Allow,
        (_, Test) => Allow,

        (Mode::Ask, Lint) | (Mode::Ask, Build) => Ask,
        (_, Lint) | (_, Build) => Allow,

        (Mode::Ask, Write) => Ask,
        (_, Write) => Allow,

        (Mode::Full, Delete)
        | (Mode::Full, Shell)
        | (Mode::Full, Network)
        | (Mode::Full, GitCommit) => Allow,
        (_, Delete) | (_, Shell) | (_, Network) | (_, GitCommit) => Ask,
    }
}

/// Live policy for a session: the mode plus grants the user made with
/// "allow for this session".
#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    pub mode: Mode,
    session_grants: BTreeSet<ActionKind>,
}

impl PermissionPolicy {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            session_grants: BTreeSet::new(),
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.session_grants.clear();
    }

    pub fn decide(&self, kinds: &[ActionKind]) -> PolicyDecision {
        let mut worst = PolicyDecision::Allow;
        for &kind in kinds {
            let mut d = mode_decision(self.mode, kind);
            if d == PolicyDecision::Ask
                && !kind.is_hard_boundary()
                && self.session_grants.contains(&kind)
            {
                d = PolicyDecision::Allow;
            }
            worst = worst.max(d);
        }
        worst
    }

    /// Remember an approval for the rest of the session. Hard boundaries are
    /// refused: returns false and records nothing.
    pub fn grant_session(&mut self, kinds: &[ActionKind]) -> bool {
        if kinds.iter().any(|k| k.is_hard_boundary()) {
            return false;
        }
        self.session_grants.extend(kinds.iter().copied());
        true
    }

    pub fn session_grants(&self) -> impl Iterator<Item = ActionKind> + '_ {
        self.session_grants.iter().copied()
    }

    /// Human-readable table for `/permissions`.
    pub fn table(&self) -> Vec<(ActionKind, PolicyDecision, bool)> {
        ActionKind::ALL
            .iter()
            .map(|&k| {
                let granted = self.session_grants.contains(&k);
                (k, self.decide(&[k]), granted)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ActionKind::*;
    use PolicyDecision::*;

    #[test]
    fn ask_mode_matches_spec() {
        let m = Mode::Ask;
        for k in [Read, Search, GitRead, Test] {
            assert_eq!(mode_decision(m, k), Allow, "{k:?}");
        }
        for k in [Write, Shell, Delete, Network, GitCommit, Lint, Build] {
            assert_eq!(mode_decision(m, k), Ask, "{k:?}");
        }
        assert_eq!(mode_decision(m, GitPush), Deny);
    }

    #[test]
    fn workspace_mode_matches_spec() {
        let m = Mode::Workspace;
        for k in [Read, Search, GitRead, Write, Test, Build, Lint] {
            assert_eq!(mode_decision(m, k), Allow, "{k:?}");
        }
        for k in [
            Shell,
            Delete,
            Destructive,
            Network,
            GitCommit,
            GitPush,
            OutsideWorkspace,
            Secrets,
        ] {
            assert_eq!(mode_decision(m, k), Ask, "{k:?}");
        }
    }

    #[test]
    fn full_mode_allows_broad_execution_but_not_hard_boundaries() {
        let m = Mode::Full;
        for k in [Write, Shell, Delete, Network, GitCommit, Build] {
            assert_eq!(mode_decision(m, k), Allow, "{k:?}");
        }
        for k in [Secrets, Privileged, OutsideWorkspace, Destructive, GitPush] {
            assert_eq!(mode_decision(m, k), Ask, "{k:?}");
        }
    }

    #[test]
    fn no_mode_silently_allows_hard_boundaries() {
        for m in Mode::ALL {
            for k in ActionKind::ALL
                .iter()
                .copied()
                .filter(|k| k.is_hard_boundary())
            {
                assert_ne!(mode_decision(m, k), Allow, "{m:?} {k:?}");
            }
        }
    }

    #[test]
    fn mode_names_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        assert_eq!(Mode::parse("autonomous"), None);
    }

    #[test]
    fn combined_decision_takes_the_strictest() {
        let policy = PermissionPolicy::new(Mode::Full);
        assert_eq!(policy.decide(&[Read, Shell]), Allow);
        assert_eq!(policy.decide(&[Shell, Secrets]), Ask);
        let safe = PermissionPolicy::new(Mode::Ask);
        assert_eq!(safe.decide(&[Read, GitPush]), Deny);
    }

    #[test]
    fn session_grants_never_cover_hard_boundaries() {
        let mut policy = PermissionPolicy::new(Mode::Workspace);
        assert_eq!(policy.decide(&[Network]), Ask);
        assert!(policy.grant_session(&[Network]));
        assert_eq!(policy.decide(&[Network]), Allow);

        assert!(!policy.grant_session(&[GitPush]));
        assert!(!policy.grant_session(&[Shell, Secrets]));
        assert_eq!(policy.decide(&[GitPush]), Ask);
        assert_eq!(policy.decide(&[Shell]), Ask, "partial grant must not leak");
    }
}
