//! `kara privacy`: a report derived from the *effective* configuration, so it
//! cannot drift from what Kara actually does.

use crate::config::{endpoint_is_local, Config, ProviderKind};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PrivacyReport {
    /// Inference happens on a machine other than this one.
    pub remote_inference: bool,
    pub telemetry: bool,
    pub account_required: bool,
    pub prompt_uploads: bool,
    pub repository_uploads: bool,
    pub inference: String,
    pub inference_endpoint: String,
    pub training_collection: String,
    pub network_uses: Vec<String>,
}

impl PrivacyReport {
    pub fn from_config(config: &Config) -> Self {
        let (inference, endpoint, remote) = match config.inference.provider {
            ProviderKind::Local => (
                "this machine (local runtime managed by Kara)".to_string(),
                config.inference.local.bind_host.clone(),
                !crate::config::is_loopback_host(&config.inference.local.bind_host),
            ),
            other => {
                let ep = config.endpoint().unwrap_or_default();
                let remote = !ep.is_empty() && !endpoint_is_local(&ep);
                let where_ = if remote {
                    format!("another machine ({})", other.label())
                } else {
                    format!("this machine ({})", other.label())
                };
                (where_, ep, remote)
            }
        };
        Self {
            remote_inference: remote,
            // There is no telemetry client in Kara, regardless of the setting.
            telemetry: false,
            account_required: false,
            prompt_uploads: remote,
            repository_uploads: remote,
            inference,
            inference_endpoint: endpoint,
            training_collection: if config.privacy.training_data {
                "enabled (local files in Kara's data directory; never uploaded by Kara)".into()
            } else {
                "disabled".into()
            },
            network_uses: vec![
                "local runtime download from github.com (only if you choose local inference; you are asked first)".into(),
                "model download from huggingface.co (only when you approve a download)".into(),
                "inference requests to the endpoint above, when it is not this machine".into(),
            ],
        }
    }

    pub fn lines(&self) -> Vec<(String, String)> {
        let yn = |b: bool| if b { "yes" } else { "no" }.to_string();
        let ed = |b: bool| if b { "ENABLED" } else { "disabled" }.to_string();
        vec![
            ("Remote inference".into(), ed(self.remote_inference)),
            ("Telemetry".into(), ed(self.telemetry)),
            ("Account required".into(), yn(self.account_required)),
            ("Prompt uploads".into(), yn(self.prompt_uploads)),
            ("Repository uploads".into(), yn(self.repository_uploads)),
            ("Inference runs on".into(), self.inference.clone()),
            ("Inference endpoint".into(), self.inference_endpoint.clone()),
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
        assert!(!r.remote_inference);
        assert!(!r.telemetry);
        assert!(!r.account_required);
        assert!(!r.prompt_uploads);
        assert!(!r.repository_uploads);
        assert_eq!(r.inference_endpoint, "127.0.0.1");
        assert_eq!(r.training_collection, "disabled");
    }

    #[test]
    fn remote_endpoint_is_reported_honestly() {
        let mut c = Config::default();
        c.inference.provider = ProviderKind::OpenaiCompat;
        c.inference.endpoint = "https://inference.example.com/v1".into();
        let r = PrivacyReport::from_config(&c);
        assert!(r.remote_inference);
        assert!(r.prompt_uploads);
        assert!(r.inference.starts_with("another machine"));
    }

    #[test]
    fn remote_kara_machine_is_reported() {
        let mut c = Config::default();
        c.inference.provider = ProviderKind::Kara;
        c.inference.endpoint = "http://192.168.1.20:7878/v1".into();
        let r = PrivacyReport::from_config(&c);
        assert!(r.remote_inference);
        assert!(r.inference.contains("Kara machine"));
    }

    #[test]
    fn ollama_default_is_local() {
        let mut c = Config::default();
        c.inference.provider = ProviderKind::Ollama;
        let r = PrivacyReport::from_config(&c);
        assert!(!r.remote_inference);
        assert_eq!(r.inference_endpoint, "http://127.0.0.1:11434/v1");
    }
}
