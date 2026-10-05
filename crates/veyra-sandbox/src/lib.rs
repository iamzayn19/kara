//! Safety primitives for Veyra.
//!
//! * [`workspace`]: confine file access to the project root, resolving `..`
//!   and symlinks before deciding whether a path is inside.
//! * [`command`]: classify shell commands into permission categories and
//!   detect dangerous operations.
//! * [`secrets`]: recognise secret-bearing paths and redact credentials from
//!   tool output before it reaches the model.
//! * [`injection`]: flag repository text that looks like instructions aimed at
//!   the agent. Detection is advisory: the real defence is that repository
//!   content can never widen permissions.

pub mod command;
pub mod injection;
pub mod secrets;
pub mod workspace;

pub use command::{classify_command, CommandAssessment, CommandContext};
pub use workspace::{PathError, ResolvedPath, Workspace};
