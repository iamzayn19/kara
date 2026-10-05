//! The Kara agent.
//!
//! [`Agent`] runs the engineering loop over a [`kara_model::ModelProvider`]
//! and the structured tools from `kara-tools`, enforcing the permission
//! policy from `kara-core`. Front ends (terminal UI, VS Code bridge) observe
//! [`kara_protocol::AgentEvent`]s and answer permission requests through an
//! [`approver::Approver`].

pub mod agent;
pub mod approver;
pub mod eval;
pub mod prompts;
pub mod session;

pub use agent::{Agent, AgentSettings, EventFn, TurnResult, TurnStats};
pub use approver::Approver;
