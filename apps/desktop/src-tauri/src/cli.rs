//! The few `kara` CLI commands the desktop app runs directly (outside the
//! `kara serve` session): pointing inference at another machine or at an
//! OpenAI-compatible server. Kept to fixed commands and config keys rather
//! than arbitrary arguments from the window.

use std::process::{Command, Stdio};
use tauri::AppHandle;

/// Config keys the window may set.
const SETTABLE: &[&str] = &[
    "inference.provider",
    "inference.endpoint",
    "inference.model",
];

fn run(app: &AppHandle, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let binary = crate::rpc::locate_kara_binary(app)
        .ok_or_else(|| "the `kara` binary was not found (bundled or on PATH)".to_string())?;
    let mut cmd = Command::new(binary);
    cmd.args(args)
        .envs(env.iter().copied())
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("could not run kara: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    if out.status.success() {
        Ok(text)
    } else {
        Err(text.lines().last().unwrap_or("kara failed").to_string())
    }
}

/// `kara connect <url>`, with the token in the environment rather than on
/// the command line, where other local users could read it from the
/// process list.
pub fn connect(app: &AppHandle, url: &str, token: &str) -> Result<String, String> {
    if url.trim().is_empty() || token.trim().is_empty() {
        return Err("a URL and a token are required".into());
    }
    run(
        app,
        &["connect", url.trim()],
        &[("KARA_REMOTE_TOKEN", token.trim())],
    )
}

fn check_settable(key: &str) -> Result<(), String> {
    if SETTABLE.contains(&key) {
        Ok(())
    } else {
        Err(format!("{key} can't be changed from the app"))
    }
}

pub fn config_set(app: &AppHandle, key: &str, value: &str) -> Result<String, String> {
    check_settable(key)?;
    run(app, &["config", "set", key, value], &[])
}

#[cfg(test)]
mod tests {
    use super::check_settable;

    #[test]
    fn only_the_inference_target_keys_are_settable() {
        for key in [
            "inference.provider",
            "inference.endpoint",
            "inference.model",
        ] {
            assert!(check_settable(key).is_ok(), "{key}");
        }
        // Permission mode and API key sources stay with the user's own
        // config edits, never the window.
        for key in [
            "permissions.mode",
            "inference.api_key_file",
            "inference.api_key_env",
            "privacy.training_data",
            "",
        ] {
            assert!(check_settable(key).is_err(), "{key}");
        }
    }
}
