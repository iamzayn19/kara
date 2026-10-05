//! Repository intelligence.
//!
//! Kara never dumps a repository into the model. Instead it builds a small,
//! incrementally maintained index (files, symbols, imports, tests), discovers
//! project commands from language packs, and ranks files for a task so the
//! model starts from a short orientation and reads only what it needs.

pub mod git;
pub mod index;
pub mod languages;
pub mod project;
pub mod rank;
pub mod search;
pub mod symbols;

pub use index::{IndexStats, RepoIndex, RepoStats, SymbolHit};
pub use languages::{LanguagePack, LanguageRegistry};
pub use project::{CommandCategory, ProjectCommand, ProjectProfile};
pub use rank::{orient, Orientation, RankedFile};

/// Bytes inspected when deciding whether a file is binary.
pub const BINARY_SNIFF_BYTES: usize = 8192;

/// Heuristic binary check: a NUL byte in the first few KiB.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0)
}
