//! Shared harness for end-to-end tests: copy a fixture repository into a
//! temporary git repo, build an agent over a scripted model, and collect the
//! events it emits.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use kara_agent::approver::Approver;
use kara_agent::{Agent, AgentSettings};
use kara_context::{LanguageRegistry, ProjectProfile, RepoIndex};
use kara_core::permissions::{PermissionPolicy, Profile};
use kara_model::ModelProvider;
use kara_protocol::AgentEvent;
use kara_sandbox::Workspace;
use kara_tools::{Journal, ToolContext};

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/repos")
}

#[derive(Debug, Deserialize, Clone)]
pub struct FixtureTask {
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
}

impl FixtureTask {
    pub fn check_command(&self) -> &str {
        if cfg!(windows) {
            self.check_windows.as_deref().unwrap_or(&self.check)
        } else {
            &self.check
        }
    }
}

#[derive(Debug, Deserialize)]
struct TaskFile {
    task: Vec<FixtureTask>,
}

pub fn fixture_tasks(name: &str) -> Vec<FixtureTask> {
    let text = std::fs::read_to_string(fixtures_dir().join(name).join("kara-tasks.toml")).unwrap();
    toml::from_str::<TaskFile>(&text).unwrap().task
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name == "solutions"
            || name == "target"
            || name == "kara-tasks.toml"
            || name == "__pycache__"
        {
            continue;
        }
        let to = dst.join(&name);
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A fixture copied into a fresh git repository.
pub struct Repo {
    pub dir: tempfile::TempDir,
}

impl Repo {
    pub fn from_fixture(name: &str) -> Repo {
        let dir = tempfile::tempdir().unwrap();
        copy_dir(&fixtures_dir().join(name), dir.path());
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "fixture@example.com"]);
        git(dir.path(), &["config", "user.name", "fixture"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "fixture"]);
        Repo { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path().join(rel)).unwrap()
    }

    pub fn write(&self, rel: &str, text: &str) {
        std::fs::write(self.path().join(rel), text).unwrap();
    }

    /// Run a shell command in the repo; returns (success, combined output).
    pub fn run(&self, cmd: &str) -> (bool, String) {
        let out = if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", cmd])
                .current_dir(self.path())
                .output()
                .unwrap()
        } else {
            Command::new("sh")
                .args(["-c", cmd])
                .current_dir(self.path())
                .output()
                .unwrap()
        };
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }
}

pub struct Harness {
    pub agent: Agent,
    pub events: Arc<Mutex<Vec<AgentEvent>>>,
    _state: tempfile::TempDir,
}

impl Harness {
    pub fn new(
        repo: &Repo,
        provider: Arc<dyn ModelProvider>,
        profile: Profile,
        approver: Arc<dyn Approver>,
    ) -> Harness {
        Self::with_settings(repo, provider, profile, approver, AgentSettings::default())
    }

    pub fn with_settings(
        repo: &Repo,
        provider: Arc<dyn ModelProvider>,
        profile: Profile,
        approver: Arc<dyn Approver>,
        settings: AgentSettings,
    ) -> Harness {
        let state = tempfile::tempdir().unwrap();
        let ws = Workspace::new(repo.path()).unwrap();
        let registry = LanguageRegistry::builtin();
        let index = RepoIndex::open(
            ws.root(),
            &state.path().join("cache"),
            registry.clone(),
            1_000_000,
        )
        .unwrap();
        index.refresh().unwrap();
        let profile_detected = ProjectProfile::detect(ws.root(), &registry);
        let mut journal = Journal::open(ws.root(), &state.path().join("session")).unwrap();
        journal.set_baseline_dirty(kara_context::git::dirty_paths(ws.root()));
        let ctx = ToolContext {
            workspace: ws,
            profile: profile_detected,
            index: Some(Arc::new(index)),
            journal: Arc::new(Mutex::new(journal)),
            output_chars: 12_000,
            cancel: tokio_util::sync::CancellationToken::new(),
            allow_commands: vec![],
            deny_commands: vec![],
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let agent = Agent::new(
            provider,
            ctx,
            PermissionPolicy::new(profile),
            approver,
            Arc::new(move |e| ev.lock().unwrap().push(e)),
            settings,
        );
        Harness {
            agent,
            events,
            _state: state,
        }
    }

    pub fn events(&self) -> Vec<AgentEvent> {
        self.events.lock().unwrap().clone()
    }
}

/// Whether a toolchain command is available (fixtures for missing
/// toolchains are skipped, not failed).
pub fn have(cmd: &str, arg: &str) -> bool {
    Command::new(cmd)
        .arg(arg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn python() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}
