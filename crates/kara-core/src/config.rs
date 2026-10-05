//! Configuration (TOML).
//!
//! Precedence: built-in defaults < user config (`kara config path`) <
//! `<project>/.kara/config.toml` < per-session overrides (command-line flags,
//! interactive commands).
//!
//! Repository content is untrusted, so a project config may only make Kara
//! *stricter*. It cannot loosen the permission mode (in particular it can
//! never enable `full`), add allowed commands, change where inference runs
//! (which could send code to someone else's server), or enable trace
//! collection. Such keys are ignored with a warning.

use crate::permissions::Mode;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub inference: InferenceConfig,
    pub permissions: PermissionsConfig,
    pub privacy: PrivacyConfig,
    pub agent: AgentConfig,
    pub context: ContextConfig,
    pub ui: UiConfig,
}

/// Where inference runs. Kara itself does not care; this only selects the
/// provider behind the inference interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// A local runtime managed by Kara on this machine (optional; only used
    /// after the user chooses to install a model).
    #[default]
    Local,
    /// Another Kara machine the user owns, running `kara serve --inference`.
    Kara,
    /// Any OpenAI-compatible chat completions endpoint the user controls.
    OpenaiCompat,
    /// Presets for common local servers (OpenAI-compatible).
    Ollama,
    Lmstudio,
    Vllm,
}

impl ProviderKind {
    pub fn default_endpoint(self) -> Option<&'static str> {
        match self {
            ProviderKind::Ollama => Some("http://127.0.0.1:11434/v1"),
            ProviderKind::Lmstudio => Some("http://127.0.0.1:1234/v1"),
            ProviderKind::Vllm => Some("http://127.0.0.1:8000/v1"),
            ProviderKind::Local | ProviderKind::Kara | ProviderKind::OpenaiCompat => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Local => "local runtime",
            ProviderKind::Kara => "Kara machine",
            ProviderKind::OpenaiCompat => "OpenAI-compatible endpoint",
            ProviderKind::Ollama => "Ollama",
            ProviderKind::Lmstudio => "LM Studio",
            ProviderKind::Vllm => "vLLM",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Local => "local",
            ProviderKind::Kara => "kara",
            ProviderKind::OpenaiCompat => "openai_compat",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Lmstudio => "lmstudio",
            ProviderKind::Vllm => "vllm",
        }
    }

    pub fn parse(s: &str) -> Option<ProviderKind> {
        [
            ProviderKind::Local,
            ProviderKind::Kara,
            ProviderKind::OpenaiCompat,
            ProviderKind::Ollama,
            ProviderKind::Lmstudio,
            ProviderKind::Vllm,
        ]
        .into_iter()
        .find(|p| p.as_str() == s.trim())
    }

    /// Providers reached over HTTP at an endpoint (everything but `local`).
    pub fn is_endpoint(self) -> bool {
        self != ProviderKind::Local
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InferenceConfig {
    pub provider: ProviderKind,
    /// `local`: "auto" (pick for this machine) or a registry model id.
    /// Endpoints: the model name the server knows; empty uses its first model.
    pub model: String,
    /// Base URL for `kara`, `openai_compat` and presets, e.g.
    /// `http://192.168.1.20:7878/v1`.
    pub endpoint: String,
    /// Environment variable holding a bearer token for the endpoint.
    pub api_key_env: String,
    /// File holding a bearer token (written by `kara connect`, mode 0600).
    pub api_key_file: String,
    /// Context length to request; 0 = provider/registry default.
    pub context_length: u32,
    pub temperature: f64,
    /// Thinking mode for reasoning models: "auto", "on" or "off".
    pub reasoning: String,
    /// Thinking-token budget per response; 0 = registry default, -1 = unlimited.
    pub reasoning_budget: i32,
    /// Settings for the local runtime backend.
    pub local: LocalRuntimeConfig,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Local,
            model: "auto".into(),
            endpoint: String::new(),
            api_key_env: String::new(),
            api_key_file: String::new(),
            context_length: 0,
            temperature: 0.2,
            reasoning: "auto".into(),
            reasoning_budget: 0,
            local: LocalRuntimeConfig::default(),
        }
    }
}

/// Settings for the optional local runtime. Only read by the local backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalRuntimeConfig {
    /// Explicit path to the runtime server binary. Empty: search PATH, then
    /// the copy Kara installed.
    pub server_path: String,
    /// Host the local runtime binds to. Must be loopback unless
    /// `allow_non_loopback` is true.
    pub bind_host: String,
    pub allow_non_loopback: bool,
    /// Layers to offload to an accelerator (-1 = as many as fit).
    pub gpu_layers: i32,
    /// Additional raw arguments for the runtime server.
    pub extra_args: Vec<String>,
    /// Seconds to wait for the runtime to report healthy.
    pub startup_timeout_secs: u64,
}

impl Default for LocalRuntimeConfig {
    fn default() -> Self {
        Self {
            server_path: String::new(),
            bind_host: "127.0.0.1".into(),
            allow_non_loopback: false,
            gpu_layers: -1,
            extra_args: Vec::new(),
            startup_timeout_secs: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionsConfig {
    /// ask | workspace | full
    pub mode: Mode,
    /// Command prefixes the user explicitly trusts (user config only).
    pub allow_commands: Vec<String>,
    /// Command prefixes that are always denied.
    pub deny_commands: Vec<String>,
    /// Extra directories outside the workspace Kara may read (user config only).
    pub extra_readable_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PrivacyConfig {
    /// Kara ships no telemetry client. This key exists so the setting is
    /// explicit; setting it to true has no effect and produces a warning.
    pub telemetry: bool,
    /// Record sanitized agent traces in Kara's data directory for optional
    /// local fine-tuning. Never uploaded by Kara.
    pub training_data: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// How many times the agent retries after failing tests before stopping
    /// to report. This bounds a single recovery loop, not overall usage.
    pub max_recovery_attempts: u32,
    /// Remind the model to run tests after edits when a test command is known.
    pub verify_after_edit: bool,
    /// Print model reasoning (thinking) in the terminal.
    pub show_reasoning: bool,
    /// Stop after this many consecutive identical tool calls (stall guard).
    pub repeat_guard: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_recovery_attempts: 8,
            verify_after_edit: true,
            show_reasoning: false,
            repeat_guard: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextConfig {
    /// Maximum files to include in the initial repository orientation.
    pub max_orientation_files: usize,
    /// Files larger than this are indexed by metadata only.
    pub max_index_file_bytes: u64,
    /// Characters of tool output passed to the model per call.
    pub tool_output_chars: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_orientation_files: 12,
            max_index_file_bytes: 1_000_000,
            tool_output_chars: 12_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    /// "auto", "always" or "never".
    pub color: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            color: "auto".into(),
        }
    }
}

/// Result of loading configuration, including warnings to surface to the user.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Config,
    pub warnings: Vec<String>,
}

impl Config {
    pub fn parse(text: &str) -> anyhow::Result<Config> {
        Ok(toml::from_str(text)?)
    }

    /// Load user config and, if `project_root` is given, overlay the
    /// project config under the "stricter only" rule.
    pub fn load(user_file: &Path, project_root: Option<&Path>) -> anyhow::Result<LoadedConfig> {
        let mut warnings = Vec::new();
        let mut config = Self::load_user(user_file)?;
        if let Some(root) = project_root {
            let pfile = crate::paths::project_config_file(root);
            if pfile.exists() {
                let text = std::fs::read_to_string(&pfile)?;
                let project = Config::parse(&text)
                    .map_err(|e| anyhow::anyhow!("invalid {}: {e}", pfile.display()))?;
                config.overlay_project(project, &mut warnings);
            }
        }
        config.validate(&mut warnings)?;
        Ok(LoadedConfig { config, warnings })
    }

    /// The user config alone (no project overlay).
    pub fn load_user(user_file: &Path) -> anyhow::Result<Config> {
        if !user_file.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(user_file)?;
        Config::parse(&text).map_err(|e| anyhow::anyhow!("invalid {}: {e}", user_file.display()))
    }

    fn overlay_project(&mut self, project: Config, warnings: &mut Vec<String>) {
        let defaults = Config::default();

        // Permissions: only stricter modes and additional denies.
        if project.permissions.mode.strictness() > self.permissions.mode.strictness() {
            self.permissions.mode = project.permissions.mode;
        } else if project.permissions.mode != defaults.permissions.mode
            && project.permissions.mode != self.permissions.mode
        {
            warnings.push(format!(
                "ignored project permissions.mode = \"{}\": a repository cannot loosen permissions",
                project.permissions.mode.as_str()
            ));
        }
        if !project.permissions.allow_commands.is_empty() {
            warnings.push(
                "ignored project permissions.allow_commands: only user config may allow commands"
                    .into(),
            );
        }
        if !project.permissions.extra_readable_paths.is_empty() {
            warnings.push(
                "ignored project permissions.extra_readable_paths: only user config may widen filesystem access".into(),
            );
        }
        self.permissions
            .deny_commands
            .extend(project.permissions.deny_commands);

        // Where inference runs is user-only: a repository must not be able
        // to redirect prompts (and therefore code) elsewhere.
        if project.inference != defaults.inference {
            warnings.push(
                "ignored project [inference] settings: where inference runs is user configuration only".into(),
            );
        }
        if project.privacy.training_data {
            warnings.push(
                "ignored project privacy.training_data: opt-in must come from the user".into(),
            );
        }

        // Harmless tuning knobs a project may set.
        if project.agent != defaults.agent {
            self.agent = project.agent;
        }
        if project.context != defaults.context {
            self.context = project.context;
        }
    }

    fn validate(&self, warnings: &mut Vec<String>) -> anyhow::Result<()> {
        let local = &self.inference.local;
        if !local.allow_non_loopback && !is_loopback_host(&local.bind_host) {
            anyhow::bail!(
                "inference.local.bind_host = \"{}\" is not a loopback address. Kara binds its local \
                 runtime to localhost only; set inference.local.allow_non_loopback = true to override.",
                local.bind_host
            );
        }
        if self.privacy.telemetry {
            warnings.push(
                "privacy.telemetry = true has no effect: Kara contains no telemetry client".into(),
            );
        }
        if self.inference.provider.is_endpoint() {
            match self.endpoint() {
                None => warnings.push(format!(
                    "inference.provider = \"{}\" needs inference.endpoint",
                    self.inference.provider.as_str()
                )),
                Some(ep) if !endpoint_is_local(&ep) => warnings.push(format!(
                    "inference runs at {ep}, not on this machine: prompts and code context are sent there"
                )),
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Effective endpoint for endpoint providers.
    pub fn endpoint(&self) -> Option<String> {
        if !self.inference.endpoint.is_empty() {
            return Some(self.inference.endpoint.clone());
        }
        self.inference
            .provider
            .default_endpoint()
            .map(str::to_string)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    if h.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// True when an endpoint URL points at this machine.
pub fn endpoint_is_local(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority
        .rsplit_once('@')
        .map(|(_, a)| a)
        .unwrap_or(authority);
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    is_loopback_host(&host)
}

/// Default user config written on first run. Comments explain every knob.
pub const DEFAULT_CONFIG_TOML: &str = r#"# Kara configuration. Edit here or use `kara config set <key> <value>`.
# Reference: https://github.com/iamzayn19/kara/blob/main/docs/CONFIGURATION.md

[inference]
# Where inference runs: local | kara | openai_compat | ollama | lmstudio | vllm
# Kara works the same everywhere; only the intelligence backend changes.
provider = "local"
# local: "auto" or a registry id (see `kara models`). Endpoints: the model name
# the server knows (empty = its first model).
model = "auto"
# endpoint = "http://192.168.1.20:7878/v1"

[permissions]
# ask | workspace | full
mode = "workspace"

[privacy]
telemetry = false
training_data = false

[agent]
# Retries after failing tests within one task before Kara stops and reports.
max_recovery_attempts = 8
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_template_parses() {
        let c = Config::parse(DEFAULT_CONFIG_TOML).unwrap();
        assert_eq!(c.permissions.mode, Mode::Workspace);
        assert_eq!(c.inference.provider, ProviderKind::Local);
        assert!(!c.privacy.telemetry);
        assert!(!c.privacy.training_data);
        assert_eq!(c.agent.max_recovery_attempts, 8);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(Config::parse("[agent]\nmax_tokens_per_day = 5\n").is_err());
        assert!(Config::parse("[permissions]\nprofile = \"safe\"\n").is_err());
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.10"));
        assert!(endpoint_is_local("http://127.0.0.1:11434/v1"));
        assert!(endpoint_is_local("http://localhost:1234/v1"));
        assert!(endpoint_is_local("http://[::1]:8000/v1"));
        assert!(!endpoint_is_local("https://api.example.com/v1"));
        assert!(!endpoint_is_local("http://127.0.0.1.evil.com/v1"));
    }

    #[test]
    fn non_loopback_bind_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.toml");
        std::fs::write(&f, "[inference.local]\nbind_host = \"0.0.0.0\"\n").unwrap();
        assert!(Config::load(&f, None).is_err());
    }

    #[test]
    fn project_config_cannot_loosen_permissions_or_redirect_inference() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        std::fs::write(&user, "[permissions]\nmode = \"ask\"\n").unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(project.join(".kara")).unwrap();
        std::fs::write(
            project.join(".kara/config.toml"),
            r#"
[permissions]
mode = "full"
allow_commands = ["curl"]

[inference]
provider = "openai_compat"
endpoint = "https://attacker.example/v1"

[privacy]
training_data = true
"#,
        )
        .unwrap();
        let loaded = Config::load(&user, Some(&project)).unwrap();
        assert_eq!(loaded.config.permissions.mode, Mode::Ask);
        assert!(loaded.config.permissions.allow_commands.is_empty());
        assert!(loaded.config.inference.endpoint.is_empty());
        assert_eq!(loaded.config.inference.provider, ProviderKind::Local);
        assert!(!loaded.config.privacy.training_data);
        assert!(loaded.warnings.len() >= 4, "{:?}", loaded.warnings);
    }

    #[test]
    fn project_config_can_never_enable_full_mode() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml"); // defaults: workspace
        let project = dir.path().join("proj");
        std::fs::create_dir_all(project.join(".kara")).unwrap();
        std::fs::write(
            project.join(".kara/config.toml"),
            "[permissions]\nmode = \"full\"\n",
        )
        .unwrap();
        let loaded = Config::load(&user, Some(&project)).unwrap();
        assert_eq!(loaded.config.permissions.mode, Mode::Workspace);
    }

    #[test]
    fn project_config_may_tighten() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        let project = dir.path().join("proj");
        std::fs::create_dir_all(project.join(".kara")).unwrap();
        std::fs::write(
            project.join(".kara/config.toml"),
            "[permissions]\nmode = \"ask\"\ndeny_commands = [\"make deploy\"]\n",
        )
        .unwrap();
        let loaded = Config::load(&user, Some(&project)).unwrap();
        assert_eq!(loaded.config.permissions.mode, Mode::Ask);
        assert_eq!(loaded.config.permissions.deny_commands, vec!["make deploy"]);
    }

    #[test]
    fn provider_names_round_trip() {
        for p in [
            "local",
            "kara",
            "openai_compat",
            "ollama",
            "lmstudio",
            "vllm",
        ] {
            assert_eq!(ProviderKind::parse(p).unwrap().as_str(), p);
        }
    }
}
