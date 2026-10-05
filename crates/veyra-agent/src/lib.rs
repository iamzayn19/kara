//! The Veyra agent.
//!
//! [`Agent`] runs the engineering loop over a [`veyra_model::ModelProvider`]
//! and the structured tools from `veyra-tools`, enforcing the permission
//! policy from `veyra-core`. Front ends (terminal UI, VS Code bridge) observe
//! [`veyra_protocol::AgentEvent`]s and answer permission requests through an
//! [`approver::Approver`].

pub mod agent;
pub mod approver;
pub mod prompts;
pub mod session;

pub use agent::{Agent, AgentSettings, EventFn, TurnResult, TurnStats};
pub use approver::Approver;
