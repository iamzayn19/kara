//! Permission profiles and decisions.
//!
//! A tool invocation is classified (by `veyra-sandbox`) into one or more
//! [`ActionKind`]s. The policy combines the decisions for each kind: any deny
//! wins, then any ask, otherwise allow.
//!
//! Hard boundaries (secrets, privilege escalation, git push, destructive
//! commands, paths outside the workspace) are never `Allow` in any profile and
//! can never be remembered for the session: each occurrence needs explicit
//! consent.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use veyra_protocol::ActionKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Safe,
    #[default]
    Balanced,
    Autonomous,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Safe => "safe",
            Profile::Balanced => "balanced",
            Profile::Autonomous => "autonomous",
        }
    }

    pub fn parse(s: &str) -> Option<Profile> {
        match s.trim().to_ascii_lowercase().as_str() {
            "safe" => Some(Profile::Safe),
            "balanced" => Some(Profile::Balanced),
            "autonomous" | "auto" => Some(Profile::Autonomous),
            _ => None,
        }
    }

    /// Higher is stricter.
    pub fn strictness(self) -> u8 {
        match self {
            Profile::Safe => 2,
            Profile::Balanced => 1,
            Profile::Autonomous => 0,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Profile::Safe => "reads, search, git-read and tests run freely; every write and shell command asks",
            Profile::Balanced => "project edits, tests, lint and builds run freely; destructive, network, commit and push ask",
            Profile::Autonomous => "ordinary repository development runs freely; high-risk boundaries still ask",
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

/// Static profile table.
pub fn profile_decision(profile: Profile, kind: ActionKind) -> PolicyDecision {
    use ActionKind::*;
    use PolicyDecision::*;
    match (profile, kind) {
        // Hard boundaries: never silently granted.
        (_, Secrets) | (_, Privileged) | (_, OutsideWorkspace) | (_, Destructive) => Ask,
        (Profile::Safe, GitPush) => Deny,
        (_, GitPush) => Ask,

        (_, Read) | (_, Search) | (_, GitRead) => Allow,
        (_, Test) => Allow,

        (Profile::Safe, Lint) | (Profile::Safe, Build) => Ask,
        (_, Lint) | (_, Build) => Allow,

        (Profile::Safe, Write) => Ask,
        (_, Write) => Allow,

        (Profile::Autonomous, Delete) => Allow,
        (_, Delete) => Ask,

        (Profile::Autonomous, Shell) => Allow,
        (_, Shell) => Ask,

        (_, Network) => Ask,
        (_, GitCommit) => Ask,
    }
}

/// Live policy for a session: the profile plus grants the user made with
/// "allow for this session".
#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    pub profile: Profile,
    session_grants: BTreeSet<ActionKind>,
}

impl PermissionPolicy {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            session_grants: BTreeSet::new(),
        }
    }

    pub fn set_profile(&mut self, profile: Profile) {
        self.profile = profile;
        self.session_grants.clear();
    }

    pub fn decide(&self, kinds: &[ActionKind]) -> PolicyDecision {
        let mut worst = PolicyDecision::Allow;
        for &kind in kinds {
            let mut d = profile_decision(self.profile, kind);
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
    fn safe_profile_matches_spec() {
        let p = Profile::Safe;
        for k in [Read, Search, GitRead, Test] {
            assert_eq!(profile_decision(p, k), Allow, "{k:?}");
        }
        for k in [Write, Shell, Delete, Network, GitCommit] {
            assert_eq!(profile_decision(p, k), Ask, "{k:?}");
        }
        assert_eq!(profile_decision(p, GitPush), Deny);
    }

    #[test]
    fn balanced_profile_matches_spec() {
        let p = Profile::Balanced;
        for k in [Read, Search, Write, Test, Build, Lint] {
            assert_eq!(profile_decision(p, k), Allow, "{k:?}");
        }
        for k in [Destructive, Network, GitCommit, GitPush, OutsideWorkspace] {
            assert_eq!(profile_decision(p, k), Ask, "{k:?}");
        }
    }

    #[test]
    fn no_profile_silently_allows_hard_boundaries() {
        for p in [Profile::Safe, Profile::Balanced, Profile::Autonomous] {
            for k in ActionKind::ALL.iter().copied().filter(|k| k.is_hard_boundary()) {
                assert_ne!(profile_decision(p, k), Allow, "{p:?} {k:?}");
            }
        }
    }

    #[test]
    fn combined_decision_takes_the_strictest() {
        let policy = PermissionPolicy::new(Profile::Autonomous);
        assert_eq!(policy.decide(&[Read, Shell]), Allow);
        assert_eq!(policy.decide(&[Shell, Secrets]), Ask);
        let safe = PermissionPolicy::new(Profile::Safe);
        assert_eq!(safe.decide(&[Read, GitPush]), Deny);
    }

    #[test]
    fn session_grants_never_cover_hard_boundaries() {
        let mut policy = PermissionPolicy::new(Profile::Balanced);
        assert_eq!(policy.decide(&[Network]), Ask);
        assert!(policy.grant_session(&[Network]));
        assert_eq!(policy.decide(&[Network]), Allow);

        assert!(!policy.grant_session(&[GitPush]));
        assert!(!policy.grant_session(&[Shell, Secrets]));
        assert_eq!(policy.decide(&[GitPush]), Ask);
        assert_eq!(policy.decide(&[Shell]), Ask, "partial grant must not leak");
    }
}
