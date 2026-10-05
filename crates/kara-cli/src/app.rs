//! Shared setup for every entry point: configuration, workspace, index,
//! project profile, sessions and the tool context.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use kara_agent::session::SessionStore;
use kara_agent::AgentSettings;
use kara_context::{git, LanguageRegistry, ProjectProfile, RepoIndex};
use kara_core::config::{Config, DEFAULT_CONFIG_TOML};
use kara_core::permissions::Profile;
use kara_core::KaraPaths;
use kara_sandbox::Workspace;
use kara_tools::{Journal, ToolContext};

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub dir: Option<PathBuf>,
    pub profile: Option<String>,
}

/// The repository to work in: `--dir`, else the git root containing the
/// current directory, else the current directory.
pub fn workspace_root(opts: &Options) -> anyhow::Result<PathBuf> {
    let start = match &opts.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir()?,
    };
    let start =
        std::fs::canonicalize(&start).map_err(|e| anyhow::anyhow!("{}: {e}", start.display()))?;
    Ok(git::toplevel(&start).unwrap_or(start))
}

pub fn ensure_user_config(paths: &KaraPaths) -> anyhow::Result<()> {
    paths.ensure()?;
    if !paths.config_file().exists() {
        std::fs::write(paths.config_file(), DEFAULT_CONFIG_TOML)?;
    }
    Ok(())
}

pub struct App {
    pub paths: KaraPaths,
    pub config: Config,
    pub warnings: Vec<String>,
    pub root: PathBuf,
    pub index: Arc<RepoIndex>,
    pub profile: ProjectProfile,
    pub sessions: SessionStore,
    pub models: kara_model::registry::Registry,
}

impl App {
    pub fn load(opts: &Options) -> anyhow::Result<App> {
        let paths = KaraPaths::discover()?;
        ensure_user_config(&paths)?;
        let root = workspace_root(opts)?;
        let loaded = Config::load(&paths.config_file(), Some(&root))?;
        let mut config = loaded.config;
        let mut warnings = loaded.warnings;
        if let Some(p) = &opts.profile {
            config.permissions.profile = Profile::parse(p).ok_or_else(|| {
                anyhow::anyhow!("unknown profile `{p}` (safe, balanced, autonomous)")
            })?;
        }
        match config.ui.color.as_str() {
            "never" => console::set_colors_enabled(false),
            "always" => console::set_colors_enabled(true),
            _ => {}
        }
        let (registry, pack_warnings) =
            LanguageRegistry::with_user_dir(&paths.home.join("languages"));
        warnings.extend(pack_warnings);
        let index = Arc::new(RepoIndex::open(
            &root,
            &paths.cache_dir(),
            registry.clone(),
            config.context.max_index_file_bytes,
        )?);
        let profile = ProjectProfile::detect(&root, &registry);
        let sessions = SessionStore::open(&paths.database())?;
        let models = kara_model::registry::Registry::load(&paths.user_models_file())?;
        Ok(App {
            paths,
            config,
            warnings,
            root,
            index,
            profile,
            sessions,
            models,
        })
    }

    /// Refresh the index on a background thread so startup never blocks on
    /// large repositories.
    pub fn refresh_index_in_background(
        &self,
    ) -> std::thread::JoinHandle<Option<kara_context::IndexStats>> {
        let index = self.index.clone();
        std::thread::spawn(move || index.refresh().ok())
    }

    pub fn session_dir(&self, session: &str) -> PathBuf {
        self.paths.sessions_dir().join(session)
    }

    pub fn tool_context(&self, session: &str) -> anyhow::Result<ToolContext> {
        let ws = Workspace::new(&self.root)?
            .with_extra_readable(&self.config.permissions.extra_readable_paths);
        let mut journal = Journal::open(ws.root(), &self.session_dir(session))?;
        if journal.batches().is_empty() {
            journal.set_baseline_dirty(git::dirty_paths(ws.root()));
        }
        Ok(ToolContext {
            workspace: ws,
            profile: self.profile.clone(),
            index: Some(self.index.clone()),
            journal: Arc::new(Mutex::new(journal)),
            output_chars: self.config.context.tool_output_chars,
            cancel: Default::default(),
            allow_commands: self.config.permissions.allow_commands.clone(),
            deny_commands: self.config.permissions.deny_commands.clone(),
        })
    }

    pub fn agent_settings(&self, context_window: Option<u32>) -> AgentSettings {
        AgentSettings {
            max_recovery_attempts: self.config.agent.max_recovery_attempts,
            verify_after_edit: self.config.agent.verify_after_edit,
            repeat_guard: self.config.agent.repeat_guard,
            temperature: self.config.model.temperature,
            max_orientation_files: self.config.context.max_orientation_files,
            context_window: context_window.unwrap_or(32768),
            trace_dir: self
                .config
                .privacy
                .training_data
                .then(|| self.paths.traces_dir()),
        }
    }

    pub fn repo_name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string())
    }

    /// Session to use: resumed (latest for this repo) or new.
    pub fn open_session(&self, resume: bool, model: &str) -> anyhow::Result<(String, bool)> {
        if resume {
            if let Some(id) = self.sessions.latest_for(&self.root)? {
                return Ok((id, true));
            }
        }
        Ok((self.sessions.create(&self.root, model)?, false))
    }
}

pub fn display_path(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}
