//! Shared foundations for Veyra: configuration, filesystem layout, the
//! permission policy, privacy reporting and structured task state.

pub mod config;
pub mod paths;
pub mod permissions;
pub mod privacy;
pub mod state;

pub use config::Config;
pub use paths::VeyraPaths;
pub use permissions::{PermissionPolicy, PolicyDecision, Profile};

/// Version of the Veyra binary and crates.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Project identity used in user-facing output.
pub const REPOSITORY_URL: &str = "https://github.com/iamzayn19/veyra";
