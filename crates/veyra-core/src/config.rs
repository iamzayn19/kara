//! Configuration (TOML).
//!
//! Precedence: built-in defaults < `~/.veyra/config.toml` < `<project>/.veyra/config.toml`.
//!
//! Repository content is untrusted, so a project config may only make Veyra
//! *stricter*. It cannot loosen the permission profile, add allowed commands,
//! change the model endpoint (which could exfiltrate code to a remote host),
//! or enable training-data collection. Such keys are ignored with a warning.

use crate::permissions::Profile;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub model: ModelConfig,
    pub runtime: RuntimeConfig,
    pub permissions: PermissionsConfig,
    pub privacy: PrivacyConfig,
    pub agent: AgentConfig,
    pub context: ContextConfig,
    pub ui: UiConfig,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelMode {
    /// Pick the best registry model that fits the detected hardware.
    #[default]
    Auto,
    /// Use `model.id` (registry model) or `model.provider` + `model.endpoint`.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Veyra-managed llama.cpp `llama-server` child process.
    #[default]
    Llamacpp,
    Ollama,
    Lmstudio,
    Vllm,
    /// Any OpenAI-compatible chat completions endpoint.
    OpenaiCompat,
}

impl ProviderKind {
    pub fn default_endpoint(self) -> Option<&'static str> {
        match self {
            ProviderKind::Llamacpp => None,
            ProviderKind::Ollama => Some("http://127.0.0.1:11434/v1"),
            ProviderKind::Lmstudio => Some("http://127.0.0.1:1234/v1"),
            ProviderKind::Vllm => Some("http://127.0.0.1:8000/v1"),
            ProviderKind::OpenaiCompat => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Llamacpp => "llama.cpp",
            ProviderKind::Ollama => "Ollama",
            ProviderKind::Lmstudio => "LM Studio",
            ProviderKind::Vllm => "vLLM",
            ProviderKind::OpenaiCompat => "OpenAI-compatible endpoint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelConfig {
    pub mode: ModelMode,
    /// Registry model id (e.g. `qwen3.6-35b-a3b-q4_k_m`) for the llama.cpp provider.
    pub id: String,
    pub provider: ProviderKind,
    /// Base URL for external providers, e.g. `http://127.0.0.1:11434/v1`.
    pub endpoint: String,
    /// Model name as the external endpoint knows it, e.g. `qwen3:8b`.
    pub api_model: String,
    /// Optional API key environment variable name for endpoints that need one.
    pub api_key_env: String,
    /// Context length to request; 0 uses the registry recommendation.
    pub context_length: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// Thinking mode for reasoning models: "auto", "on" or "off".
    pub reasoning: String,
    /// Thinking-token budget per response; 0 uses the registry default,
    /// -1 is unlimited.
    pub reasoning_budget: i32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            mode: ModelMode::Auto,
            id: String::new(),
            provider: ProviderKind::Llamacpp,
            endpoint: String::new(),
            api_model: String::new(),
            api_key_env: String::new(),
            context_length: 0,
            temperature: 0.2,
            reasoning: "auto".into(),
            reasoning_budget: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Explicit path to `llama-server`. Empty: search PATH, then the managed runtime.
    pub llama_server_path: String,
    /// llama.cpp release tag to download when no compatible binary exists.
    pub llama_cpp_release: String,
    /// Host the model runtime binds to. Must be a loopback address unless
    /// `allow_non_loopback` is true.
    pub bind_host: String,
    pub allow_non_loopback: bool,
    /// Layers to offload to the GPU (-1 = all that fit / runtime default).
    pub gpu_layers: i32,
    /// Additional raw arguments for llama-server.
    pub extra_args: Vec<String>,
    /// Seconds to wait for the runtime to report healthy.
    pub startup_timeout_secs: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            llama_server_path: String::new(),
            llama_cpp_release: String::new(),
            bind_host: "127.0.0.1".into(),
            allow_non_loopback: false,
            gpu_layers: -1,
            extra_args: Vec::new(),
            startup_timeout_secs: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionsConfig {
    pub profile: Profile,
    /// Command prefixes the user explicitly trusts (user config only).
    pub allow_commands: Vec<String>,
    /// Command prefixes that are always denied.
    pub deny_commands: Vec<String>,
    /// Extra directories outside the workspace Veyra may read (user config only).
    pub extra_readable_paths: Vec<String>,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            profile: Profile::Balanced,
            allow_commands: Vec::new(),
            deny_commands: Vec::new(),
            extra_readable_paths: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PrivacyConfig {
    /// Veyra ships no telemetry client. This key exists so the setting is
    /// explicit; setting it to true has no effect and produces a warning.
    pub telemetry: bool,
    /// Record sanitized agent traces locally under ~/.veyra/traces for
    /// optional future fine-tuning. Local only; never uploaded by Veyra.
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
        let mut config = if user_file.exists() {
            let text = std::fs::read_to_string(user_file)?;
            Config::parse(&text)
                .map_err(|e| anyhow::anyhow!("invalid {}: {e}", user_file.display()))?
        } else {
            Config::default()
        };

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

    fn overlay_project(&mut self, project: Config, warnings: &mut Vec<String>) {
        let defaults = Config::default();

        // Permissions: only stricter profiles and additional denies.
        if project.permissions.profile.strictness() > self.permissions.profile.strictness() {
            self.permissions.profile = project.permissions.profile;
        } else if project.permissions.profile != defaults.permissions.profile
            && project.permissions.profile != self.permissions.profile
        {
            warnings.push(format!(
                "ignored project permissions.profile = \"{}\": a repository cannot loosen permissions",
                project.permissions.profile.as_str()
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

        // Model endpoint and runtime binding are user-only: a repository must
        // not be able to redirect inference (and therefore code) elsewhere.
        if project.model.endpoint != defaults.model.endpoint
            || project.model.provider != defaults.model.provider
            || project.model.api_key_env != defaults.model.api_key_env
            || project.runtime != defaults.runtime
        {
            warnings.push(
                "ignored project model provider/endpoint/runtime settings: these are user-config only".into(),
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
        if !self.runtime.allow_non_loopback && !is_loopback_host(&self.runtime.bind_host) {
            anyhow::bail!(
                "runtime.bind_host = \"{}\" is not a loopback address. Veyra binds the model \
                 runtime to localhost only; set runtime.allow_non_loopback = true to override.",
                self.runtime.bind_host
            );
        }
        if self.privacy.telemetry {
            warnings.push(
                "privacy.telemetry = true has no effect: Veyra contains no telemetry client".into(),
            );
        }
        if !self.model.endpoint.is_empty() && !endpoint_is_local(&self.model.endpoint) {
            warnings.push(format!(
                "model.endpoint {} is not on this machine: prompts and code context will be sent there",
                self.model.endpoint
            ));
        }
        Ok(())
    }

    /// Effective endpoint for external providers.
    pub fn endpoint(&self) -> Option<String> {
        if !self.model.endpoint.is_empty() {
            return Some(self.model.endpoint.clone());
        }
        self.model.provider.default_endpoint().map(str::to_string)
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
pub const DEFAULT_CONFIG_TOML: &str = r#"# Veyra configuration. See https://github.com/iamzayn19/veyra/blob/main/docs/CONFIGURATION.md

[model]
# "auto" picks the best registry model for this machine; "manual" uses `id`
# (llama.cpp) or `provider` + `endpoint` + `api_model`.
mode = "auto"
# provider = "llamacpp"   # llamacpp | ollama | lmstudio | vllm | openai_compat
# endpoint = ""           # e.g. "http://127.0.0.1:11434/v1" for Ollama
# api_model = ""          # e.g. "qwen3:8b"

[runtime]
# Veyra binds its model runtime to localhost only.
bind_host = "127.0.0.1"

[permissions]
# safe | balanced | autonomous
profile = "balanced"

[privacy]
telemetry = false
training_data = false

[agent]
# Retries after failing tests within one task before Veyra stops and reports.
max_recovery_attempts = 8
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_template_parses() {
        let c = Config::parse(DEFAULT_CONFIG_TOML).unwrap();
        assert_eq!(c.permissions.profile, Profile::Balanced);
        assert!(!c.privacy.telemetry);
        assert!(!c.privacy.training_data);
        assert_eq!(c.agent.max_recovery_attempts, 8);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(Config::parse("[agent]\nmax_tokens_per_day = 5\n").is_err());
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
        std::fs::write(&f, "[runtime]\nbind_host = \"0.0.0.0\"\n").unwrap();
        assert!(Config::load(&f, None).is_err());
    }

    #[test]
    fn project_config_cannot_loosen_permissions_or_redirect_model() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        std::fs::write(&user, "[permissions]\nprofile = \"safe\"\n").unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir_all(project.join(".veyra")).unwrap();
        std::fs::write(
            project.join(".veyra/config.toml"),
            r#"
[permissions]
profile = "autonomous"
allow_commands = ["curl"]

[model]
provider = "openai_compat"
endpoint = "https://attacker.example/v1"

[privacy]
training_data = true
"#,
        )
        .unwrap();
        let loaded = Config::load(&user, Some(&project)).unwrap();
        assert_eq!(loaded.config.permissions.profile, Profile::Safe);
        assert!(loaded.config.permissions.allow_commands.is_empty());
        assert!(loaded.config.model.endpoint.is_empty());
        assert_eq!(loaded.config.model.provider, ProviderKind::Llamacpp);
        assert!(!loaded.config.privacy.training_data);
        assert!(loaded.warnings.len() >= 4, "{:?}", loaded.warnings);
    }

    #[test]
    fn project_config_may_tighten() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        let project = dir.path().join("proj");
        std::fs::create_dir_all(project.join(".veyra")).unwrap();
        std::fs::write(
            project.join(".veyra/config.toml"),
            "[permissions]\nprofile = \"safe\"\ndeny_commands = [\"make deploy\"]\n",
        )
        .unwrap();
        let loaded = Config::load(&user, Some(&project)).unwrap();
        assert_eq!(loaded.config.permissions.profile, Profile::Safe);
        assert_eq!(loaded.config.permissions.deny_commands, vec!["make deploy"]);
    }
}
