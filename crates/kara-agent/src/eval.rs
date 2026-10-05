//! Offline evaluation harness (`kara eval`).
//!
//! Each task is a fixture repository plus a prompt and a check command. The
//! harness copies the fixture into a throwaway git repository, runs the agent
//! (autonomous profile; hard boundaries are always denied), then runs the
//! check command independently of anything the agent reported.
//!
//! The `oracle` provider replays each task's reference solution through the
//! real tools. It measures nothing about model quality; it proves the harness,
//! fixtures and tool path work, and runs in CI without a model.

use crate::agent::{Agent, AgentSettings, TurnResult};
use crate::approver::ApproveOrdinary;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use kara_context::{LanguageRegistry, ProjectProfile, RepoIndex};
use kara_core::permissions::{PermissionPolicy, Profile};
use kara_model::scripted::{call, text, ScriptedProvider};
use kara_model::ModelProvider;
use kara_protocol::{AgentMode, TurnOutcome};
use kara_sandbox::Workspace;
use kara_tools::{Journal, ToolContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalTask {
    pub id: String,
    pub category: String,
    pub prompt: String,
    pub check: String,
    #[serde(default)]
    pub check_windows: Option<String>,
    #[serde(default)]
    pub expect_changed: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(skip)]
    pub fixture: PathBuf,
}

impl EvalTask {
    pub fn check_command(&self) -> &str {
        if cfg!(windows) {
            self.check_windows.as_deref().unwrap_or(&self.check)
        } else {
            &self.check
        }
    }

    pub fn fixture_name(&self) -> String {
        self.fixture
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn solution_path(&self, id: &str) -> PathBuf {
        self.fixture.join("solutions").join(format!("{id}.patch"))
    }
}

#[derive(Debug, Deserialize)]
struct TaskFile {
    task: Vec<EvalTask>,
}

/// Load all tasks from `<suite>/<fixture>/kara-tasks.toml`.
pub fn load_suite(suite: &Path) -> anyhow::Result<Vec<EvalTask>> {
    let mut tasks = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(suite)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("kara-tasks.toml").exists())
        .collect();
    dirs.sort();
    for d in dirs {
        let f: TaskFile = toml::from_str(&std::fs::read_to_string(d.join("kara-tasks.toml"))?)
            .map_err(|e| anyhow::anyhow!("{}: {e}", d.display()))?;
        for mut t in f.task {
            t.fixture = d.clone();
            tasks.push(t);
        }
    }
    Ok(tasks)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub task: String,
    pub category: String,
    pub success: bool,
    pub outcome: TurnOutcome,
    pub tests_passing: Option<bool>,
    pub iterations: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub invalid_tool_calls: u32,
    pub unnecessary_edits: Vec<String>,
    pub changed_files: Vec<String>,
    pub duration_ms: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub peak_memory_bytes: Option<u64>,
    pub check_output_tail: String,
    pub error: Option<String>,
    /// Set when the task could not run here (e.g. its toolchain is missing).
    #[serde(default)]
    pub skipped: Option<String>,
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        if name == "solutions"
            || name == "kara-tasks.toml"
            || name == "target"
            || name == "node_modules"
            || name == "__pycache__"
        {
            continue;
        }
        let to = dst.join(&name);
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn shell(dir: &Path, cmd: &str) -> (bool, String) {
    let out = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", cmd])
            .current_dir(dir)
            .output()
    } else {
        Command::new("sh")
            .args(["-c", cmd])
            .current_dir(dir)
            .output()
    };
    match out {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (false, e.to_string()),
    }
}

async fn apply_patch_file(ctx: &ToolContext, patch: &Path) -> anyhow::Result<()> {
    use kara_tools::Tool;
    let text = std::fs::read_to_string(patch)?;
    let out = kara_tools::fs::ApplyPatch
        .run(&json!({"patch": text}), ctx)
        .await;
    if !out.ok {
        anyhow::bail!("{}: {}", patch.display(), out.content);
    }
    Ok(())
}

/// A prepared task workspace (temporary git repository).
pub struct Prepared {
    pub repo: tempfile::TempDir,
    pub state: tempfile::TempDir,
}

pub async fn prepare(task: &EvalTask) -> anyhow::Result<Prepared> {
    let repo = tempfile::tempdir()?;
    copy_dir(&task.fixture, repo.path())?;
    git(repo.path(), &["init", "-q", "-b", "main"])?;
    git(repo.path(), &["config", "user.email", "eval@kara.invalid"])?;
    git(repo.path(), &["config", "user.name", "kara-eval"])?;
    git(repo.path(), &["config", "commit.gpgsign", "false"])?;
    let state = tempfile::tempdir()?;
    if !task.requires.is_empty() {
        let ctx = tool_context(repo.path(), state.path(), false)?;
        for dep in &task.requires {
            apply_patch_file(&ctx, &task.solution_path(dep)).await?;
        }
    }
    git(repo.path(), &["add", "-A"])?;
    git(repo.path(), &["commit", "-qm", "fixture"])?;
    Ok(Prepared { repo, state })
}

fn tool_context(root: &Path, state: &Path, with_index: bool) -> anyhow::Result<ToolContext> {
    let ws = Workspace::new(root)?;
    let registry = LanguageRegistry::builtin();
    let index = if with_index {
        let idx = RepoIndex::open(ws.root(), &state.join("cache"), registry.clone(), 1_000_000)?;
        idx.refresh()?;
        Some(Arc::new(idx))
    } else {
        None
    };
    let profile = ProjectProfile::detect(ws.root(), &registry);
    let journal = Journal::open(ws.root(), &state.join("journal"))?;
    Ok(ToolContext {
        workspace: ws,
        profile,
        index,
        journal: Arc::new(Mutex::new(journal)),
        output_chars: 12_000,
        cancel: Default::default(),
        allow_commands: vec![],
        deny_commands: vec![],
    })
}

/// Scripted provider that applies the reference solution and verifies it.
pub fn oracle_provider(task: &EvalTask) -> anyhow::Result<ScriptedProvider> {
    let patch = std::fs::read_to_string(task.solution_path(&task.id))?;
    Ok(ScriptedProvider::new(vec![
        call("apply_patch", json!({"patch": patch})),
        call("run_test", json!({"command": task.check_command()})),
        text("Applied the reference solution and verified it with the task's check command."),
    ]))
}

/// Run one task with `provider`. `sample_memory` is polled during the run to
/// record peak runtime memory (e.g. of llama-server).
pub async fn run_task(
    task: &EvalTask,
    provider: Arc<dyn ModelProvider>,
    settings: AgentSettings,
    sample_memory: Option<Arc<dyn Fn() -> Option<u64> + Send + Sync>>,
    time_limit: Option<std::time::Duration>,
) -> EvalResult {
    let started = Instant::now();
    let mut result = EvalResult {
        task: task.id.clone(),
        category: task.category.clone(),
        success: false,
        outcome: TurnOutcome::Error,
        tests_passing: None,
        iterations: 0,
        tool_calls: 0,
        tool_errors: 0,
        invalid_tool_calls: 0,
        unnecessary_edits: vec![],
        changed_files: vec![],
        duration_ms: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        peak_memory_bytes: None,
        check_output_tail: String::new(),
        error: None,
        skipped: None,
    };
    if let Some(missing) = missing_toolchain(task.check_command()) {
        result.skipped = Some(format!("`{missing}` is not installed"));
        return result;
    }
    let prepared = match prepare(task).await {
        Ok(p) => p,
        Err(e) => {
            result.error = Some(format!("prepare failed: {e}"));
            return result;
        }
    };
    let ctx = match tool_context(prepared.repo.path(), prepared.state.path(), true) {
        Ok(c) => c,
        Err(e) => {
            result.error = Some(e.to_string());
            return result;
        }
    };

    let peak = Arc::new(Mutex::new(None::<u64>));
    let sampler = sample_memory.map(|f| {
        let peak = peak.clone();
        tokio::spawn(async move {
            loop {
                if let Some(m) = f() {
                    let mut p = peak.lock().unwrap();
                    *p = Some(p.unwrap_or(0).max(m));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        })
    });

    let mut agent = Agent::new(
        provider,
        ctx,
        PermissionPolicy::new(Profile::Autonomous),
        Arc::new(ApproveOrdinary),
        Arc::new(|_| {}),
        settings,
    );
    // A wall-clock budget per benchmark task (the agent itself has no step
    // limit; this only bounds how long one evaluation may take).
    let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let timer = time_limit.map(|limit| {
        let cancel = agent.ctx.cancel.clone();
        let flag = timed_out.clone();
        tokio::spawn(async move {
            tokio::time::sleep(limit).await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            cancel.cancel();
        })
    });
    let turn: TurnResult = agent.run_turn(&task.prompt, AgentMode::Execute).await;
    if let Some(t) = timer {
        t.abort();
    }
    if let Some(s) = sampler {
        s.abort();
    }

    let (ok, out) = shell(prepared.repo.path(), task.check_command());
    let tail: Vec<&str> = out.lines().rev().take(15).collect();
    result.check_output_tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    result.success = ok;
    result.outcome = turn.outcome;
    result.tests_passing = turn.last_test.as_ref().map(|t| t.succeeded());
    result.iterations = turn.stats.model_calls;
    result.tool_calls = turn.stats.tool_calls;
    result.tool_errors = turn.stats.tool_errors;
    result.invalid_tool_calls = turn.stats.invalid_tool_calls;
    result.unnecessary_edits = turn
        .changed_files
        .iter()
        .filter(|f| !task.expect_changed.contains(f) && !looks_like_test(f))
        .cloned()
        .collect();
    result.changed_files = turn.changed_files;
    result.prompt_tokens = turn.stats.usage.prompt_tokens;
    result.completion_tokens = turn.stats.usage.completion_tokens;
    result.peak_memory_bytes = *peak.lock().unwrap();
    result.duration_ms = started.elapsed().as_millis() as u64;
    if timed_out.load(std::sync::atomic::Ordering::SeqCst) {
        result.error = Some(format!(
            "time limit of {}s reached",
            time_limit.map(|d| d.as_secs()).unwrap_or(0)
        ));
    } else if turn.outcome == TurnOutcome::Error {
        result.error = Some(turn.summary);
    }
    result
}

/// The program a check command needs, if it is not on PATH.
pub fn missing_toolchain(check: &str) -> Option<String> {
    let prog = check.split_whitespace().next()?.to_string();
    let candidates: Vec<String> = if cfg!(windows) {
        vec![
            format!("{prog}.exe"),
            format!("{prog}.cmd"),
            format!("{prog}.bat"),
            prog.clone(),
        ]
    } else {
        vec![prog.clone()]
    };
    let path = std::env::var_os("PATH")?;
    let found =
        std::env::split_paths(&path).any(|d| candidates.iter().any(|c| d.join(c).is_file()));
    (!found).then_some(prog)
}

fn looks_like_test(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.starts_with("test/")
        || p.starts_with("tests/")
        || p.starts_with("spec/")
        || p.contains("_test.")
        || p.contains(".test.")
        || p.contains("_spec.")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    pub model: String,
    pub provider: String,
    pub started: String,
    pub kara_version: String,
    pub host: String,
    pub results: Vec<EvalResult>,
}

impl EvalReport {
    pub fn solved(&self) -> usize {
        self.results.iter().filter(|r| r.success).count()
    }

    pub fn attempted(&self) -> usize {
        self.results.iter().filter(|r| r.skipped.is_none()).count()
    }

    pub fn markdown(&self) -> String {
        let n = self.attempted();
        let skipped = self.results.len() - n;
        let mut s = format!(
            "### {} ({})\n\nSolved {}/{} ({:.0}%) on {}. Kara {}, {}.{}\n\n",
            self.model,
            self.provider,
            self.solved(),
            n,
            if n == 0 {
                0.0
            } else {
                self.solved() as f64 * 100.0 / n as f64
            },
            self.started.get(..10).unwrap_or(&self.started),
            self.kara_version,
            self.host,
            if skipped > 0 {
                format!(" {skipped} task(s) skipped (toolchain not installed).")
            } else {
                String::new()
            }
        );
        s.push_str("| Task | Category | Solved | Outcome | Model calls | Tool calls | Tool errors | Invalid calls | Unneeded edits | Time (s) | Tokens in/out | Peak mem |\n");
        s.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|\n");
        for r in self.results.iter().filter(|r| r.skipped.is_none()) {
            s.push_str(&format!(
                "| {} | {} | {} | {:?} | {} | {} | {} | {} | {} | {:.1} | {}/{} | {} |\n",
                r.task,
                r.category,
                if r.success { "yes" } else { "no" },
                r.outcome,
                r.iterations,
                r.tool_calls,
                r.tool_errors,
                r.invalid_tool_calls,
                r.unnecessary_edits.len(),
                r.duration_ms as f64 / 1000.0,
                r.prompt_tokens,
                r.completion_tokens,
                r.peak_memory_bytes
                    .map(|b| format!("{:.1} GB", b as f64 / (1u64 << 30) as f64))
                    .unwrap_or_else(|| "-".into()),
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/repos")
    }

    #[test]
    fn suite_loads() {
        let tasks = load_suite(&suite()).unwrap();
        assert!(tasks.len() >= 10, "{}", tasks.len());
        for t in &tasks {
            assert!(
                t.solution_path(&t.id).exists(),
                "{} has a reference solution",
                t.id
            );
        }
    }

    #[tokio::test]
    async fn oracle_solves_python_tasks_through_the_real_harness() {
        if Command::new(if cfg!(windows) { "python" } else { "python3" })
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tasks = load_suite(&suite()).unwrap();
        for t in tasks.iter().filter(|t| t.id.starts_with("python-")) {
            let provider = Arc::new(oracle_provider(t).unwrap());
            let r = run_task(t, provider, AgentSettings::default(), None, None).await;
            assert!(r.success, "{}: {:?} {}", t.id, r.error, r.check_output_tail);
            assert_eq!(r.tests_passing, Some(true));
            assert!(r.unnecessary_edits.is_empty(), "{:?}", r.unnecessary_edits);
        }
    }
}
