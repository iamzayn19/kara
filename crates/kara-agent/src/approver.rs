//! Permission approvers: the user-facing half of the permission system.
//! The CLI and the editor bridge implement [`Approver`] interactively.

use kara_protocol::{PermissionDecision, PermissionRequest};

#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    async fn decide(&self, request: &PermissionRequest) -> PermissionDecision;
}

/// Denies everything that needs approval. Used for non-interactive runs.
pub struct DenyAll;

#[async_trait::async_trait]
impl Approver for DenyAll {
    async fn decide(&self, _: &PermissionRequest) -> PermissionDecision {
        PermissionDecision::Deny
    }
}

/// Approves ordinary requests once but always denies hard boundaries.
/// Used by the evaluation harness inside throwaway copies of fixtures.
pub struct ApproveOrdinary;

#[async_trait::async_trait]
impl Approver for ApproveOrdinary {
    async fn decide(&self, r: &PermissionRequest) -> PermissionDecision {
        if r.kinds.iter().any(|k| k.is_hard_boundary()) {
            PermissionDecision::Deny
        } else {
            PermissionDecision::AllowOnce
        }
    }
}

/// Records requests and answers from a fixed decision (tests).
pub struct Recording {
    pub decision: PermissionDecision,
    pub seen: std::sync::Mutex<Vec<PermissionRequest>>,
}

impl Recording {
    pub fn new(decision: PermissionDecision) -> Self {
        Self {
            decision,
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl Approver for Recording {
    async fn decide(&self, r: &PermissionRequest) -> PermissionDecision {
        self.seen.lock().unwrap().push(r.clone());
        self.decision
    }
}
