//! Structured tools.
//!
//! Each tool declares a JSON schema for the model, *assesses* a call (which
//! permission categories it touches, plus a human-readable title and
//! preview) before anything happens, and only then runs. The agent owns the
//! permission decision; tools never prompt.

pub mod exec;
pub mod fs;
pub mod gittools;
pub mod journal;
pub mod output;
pub mod patch;
pub mod process;
pub mod search;
pub mod testparse;
pub mod worktree;

use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use veyra_context::{ProjectProfile, RepoIndex};
use veyra_protocol::{ActionKind, FileChange, TestReport};
use veyra_sandbox::Workspace;

pub use journal::{Journal, UndoReport};

/// Shared state tools operate on.
pub struct ToolContext {
    pub workspace: Workspace,
    pub profile: ProjectProfile,
    pub index: Option<Arc<RepoIndex>>,
    pub journal: Arc<Mutex<Journal>>,
    /// Characters of output passed back to the model per call.
    pub output_chars: usize,
    pub cancel: CancellationToken,
    pub allow_commands: Vec<String>,
    pub deny_commands: Vec<String>,
}

impl ToolContext {
    pub fn root(&self) -> &std::path::Path {
        self.workspace.root()
    }

    pub fn journal(&self) -> std::sync::MutexGuard<'_, Journal> {
        self.journal.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn known_commands(&self) -> Vec<(String, ActionKind)> {
        use veyra_context::CommandCategory as C;
        self.profile
            .known_commands()
            .into_iter()
            .filter_map(|(cmd, cat)| {
                let kind = match cat {
                    C::Test => ActionKind::Test,
                    C::Lint | C::Typecheck => ActionKind::Lint,
                    C::Build => ActionKind::Build,
                    // Formatters rewrite files.
                    C::Format => return None,
                };
                Some((cmd, kind))
            })
            .collect()
    }
}

/// What a call will do, for the permission check and the UI.
#[derive(Debug, Clone, Default)]
pub struct Assessment {
    pub kinds: Vec<ActionKind>,
    /// One-line description, e.g. `edit src/auth.rs`.
    pub title: String,
    /// Full command, path or diff preview.
    pub detail: String,
    pub reasons: Vec<String>,
    /// Set when the call must not run at all.
    pub blocked: Option<String>,
    /// Matched a user-configured allow-list entry.
    pub user_allowed: bool,
}

impl Assessment {
    pub fn new(title: impl Into<String>, kinds: Vec<ActionKind>) -> Self {
        Self {
            title: title.into(),
            kinds,
            ..Default::default()
        }
    }

    pub fn push(&mut self, kind: ActionKind, reason: impl Into<String>) {
        if !self.kinds.contains(&kind) {
            self.kinds.push(kind);
        }
        let r = reason.into();
        if !r.is_empty() && !self.reasons.contains(&r) {
            self.reasons.push(r);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub ok: bool,
    /// Text returned to the model (already truncated and redacted).
    pub content: String,
    /// One-line summary for the UI.
    pub summary: String,
    pub data: Value,
    pub changes: Vec<FileChange>,
    pub test: Option<TestReport>,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            content: content.into(),
            summary: summary.into(),
            ..Default::default()
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        let m = msg.into();
        Self {
            ok: false,
            summary: output::clip_chars(m.lines().next().unwrap_or(""), 120),
            content: format!("ERROR: {m}"),
            ..Default::default()
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON schema of the arguments object.
    fn parameters(&self) -> Value;
    /// Read-only tools are available in plan and review modes.
    fn read_only(&self) -> bool;
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String>;
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput;
}

/// All built-in tools.
pub fn builtin_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(fs::ReadFile),
        Arc::new(fs::ReadRange),
        Arc::new(fs::ListDirectory),
        Arc::new(search::FindFiles),
        Arc::new(search::Grep),
        Arc::new(search::FindSymbol),
        Arc::new(search::FindReferences),
        Arc::new(fs::EditFile),
        Arc::new(fs::InsertLines),
        Arc::new(fs::WriteFile),
        Arc::new(fs::CreateFile),
        Arc::new(fs::ApplyPatch),
        Arc::new(fs::MoveFile),
        Arc::new(fs::DeleteFile),
        Arc::new(exec::Shell),
        Arc::new(exec::RunTest),
        Arc::new(exec::RunLint),
        Arc::new(exec::RunBuild),
        Arc::new(exec::Diagnostics),
        Arc::new(gittools::GitStatus),
        Arc::new(gittools::GitDiff),
        Arc::new(gittools::GitLog),
        Arc::new(gittools::GitShow),
        Arc::new(gittools::GitBlame),
    ]
}

// ---- argument helpers -------------------------------------------------------

pub(crate) fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string argument `{key}`"))
}

pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

pub(crate) fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

pub(crate) fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| {
        v.as_bool()
            .or_else(|| v.as_str().map(|s| s.eq_ignore_ascii_case("true")))
    })
}

pub(crate) fn opt_str_list(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => s
            .split([',', ' '])
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Permission kinds for accessing a resolved path.
pub(crate) fn path_kinds(
    ctx: &ToolContext,
    p: &veyra_sandbox::ResolvedPath,
    write: bool,
    a: &mut Assessment,
) {
    if p.secret {
        a.push(
            ActionKind::Secrets,
            format!("{} looks like a credentials file", p.display()),
        );
    }
    if !p.inside && !(!write && ctx.workspace.is_extra_readable(p)) {
        a.push(
            ActionKind::OutsideWorkspace,
            format!("{} is outside the workspace", p.abs.display()),
        );
    }
    if write && p.git_internal {
        a.push(
            ActionKind::Destructive,
            "writes inside .git (could install hooks)",
        );
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    pub struct Fixture {
        pub dir: tempfile::TempDir,
        pub _state: tempfile::TempDir,
        pub ctx: ToolContext,
    }

    pub fn fixture(files: &[(&str, &str)]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let state = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let journal = Journal::open(ws.root(), state.path()).unwrap();
        let profile =
            ProjectProfile::detect(ws.root(), &veyra_context::LanguageRegistry::builtin());
        let ctx = ToolContext {
            workspace: ws,
            profile,
            index: None,
            journal: Arc::new(Mutex::new(journal)),
            output_chars: 12_000,
            cancel: CancellationToken::new(),
            allow_commands: vec![],
            deny_commands: vec![],
        };
        Fixture {
            dir,
            _state: state,
            ctx,
        }
    }
}
