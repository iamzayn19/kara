//! Local inference runtime management.
//!
//! * [`download`]: resumable HTTPS downloads verified against SHA-256.
//! * [`archive`]: safe extraction (no path traversal) of runtime archives.
//! * [`llamacpp`]: locate or install a pinned llama.cpp build.
//! * [`server`]: launch `llama-server` on 127.0.0.1, wait for health, stop it.
//! * [`store`]: where models live and what is recorded about them.
//!
//! Network access here is limited to software and model downloads that the
//! user approved. Inference itself stays on the local machine.

pub mod archive;
pub mod download;
pub mod llamacpp;
pub mod server;
pub mod store;

pub const USER_AGENT: &str = concat!(
    "kara/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/iamzayn19/kara)"
);
