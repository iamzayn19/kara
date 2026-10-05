//! `kara privacy`: a report derived from the *effective* configuration, so it
//! cannot drift from what Kara actually does.

use crate::config::{endpoint_is_local, Config, ProviderKind};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PrivacyReport {
    pub cloud_inference: bool,
    pub telemetry: bool,
    pub account_required: bool,
    pub prompt_uploads: bool,
    pub repository_uploads: bool,
    pub model_runtime: String,
    pub model_endpoint: String,
    pub training_collection: String,
    pub network_uses: Vec<String>,
}

impl PrivacyReport {
    pub fn from_config(config: &Config) -> Self {
        let (runtime, endpoint, remote) = match config.model.provider {
            ProviderKind::Llamacpp => (
                "local (llama.cpp, managed by Kara)".to_string(),
                config.runtime.bind_host.clone(),
                !crate::config::is_loopback_host(&config.runtime.bind_host),
            ),
            other => {
                let ep = config.endpoint().unwrap_or_default();
                let remote = !ep.is_empty() && !endpoint_is_local(&ep);
                let runtime = if remote {
                    format!("REMOTE ({})", other.label())
                } else {
                    format!("local ({})", other.label())
                };
                (runtime, ep, remote)
            }
        };
        Self {
            cloud_inference: remote,
            // There is no telemetry client in Kara, regardless of the setting.
            telemetry: false,
            account_required: false,
            prompt_uploads: remote,
            repository_uploads: remote,
            model_runtime: runtime,
            model_endpoint: endpoint,
            training_collection: if config.privacy.training_data {
                "enabled (local files only, ~/.kara/traces; never uploaded by Kara)".into()
            } else {
                "disabled".into()
            },
            network_uses: vec![
                "llama.cpp runtime download from github.com (only when no runtime is installed; you are asked first)".into(),
                "model download from huggingface.co (only when you approve a download)".into(),
            ],
        }
    }

    pub fn lines(&self) -> Vec<(String, String)> {
        let yn = |b: bool| if b { "yes" } else { "no" }.to_string();
        let ed = |b: bool| if b { "ENABLED" } else { "disabled" }.to_string();
        vec![
            ("Cloud inference".into(), ed(self.cloud_inference)),
            ("Telemetry".into(), ed(self.telemetry)),
            ("Account required".into(), yn(self.account_required)),
            ("Prompt uploads".into(), yn(self.prompt_uploads)),
            ("Repository uploads".into(), yn(self.repository_uploads)),
            ("Model runtime".into(), self.model_runtime.clone()),
            ("Model endpoint".into(), self.model_endpoint.clone()),
            (
                "Training collection".into(),
                self.training_collection.clone(),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_report_is_fully_local() {
        let r = PrivacyReport::from_config(&Config::default());
        assert!(!r.cloud_inference);
        assert!(!r.telemetry);
        assert!(!r.account_required);
        assert!(!r.prompt_uploads);
        assert!(!r.repository_uploads);
        assert_eq!(r.model_endpoint, "127.0.0.1");
        assert_eq!(r.training_collection, "disabled");
    }

    #[test]
    fn remote_endpoint_is_reported_honestly() {
        let mut c = Config::default();
        c.model.provider = ProviderKind::OpenaiCompat;
        c.model.endpoint = "https://inference.example.com/v1".into();
        let r = PrivacyReport::from_config(&c);
        assert!(r.cloud_inference);
        assert!(r.prompt_uploads);
        assert!(r.model_runtime.starts_with("REMOTE"));
    }

    #[test]
    fn ollama_default_is_local() {
        let mut c = Config::default();
        c.model.provider = ProviderKind::Ollama;
        let r = PrivacyReport::from_config(&c);
        assert!(!r.cloud_inference);
        assert_eq!(r.model_endpoint, "http://127.0.0.1:11434/v1");
    }
}
