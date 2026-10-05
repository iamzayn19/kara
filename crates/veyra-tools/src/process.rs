//! Running project commands: shell selection, timeouts, cancellation and
//! process-group cleanup so test runners do not leave orphans behind.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProcessResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Cap on bytes captured per stream; the rest is counted but discarded.
const CAPTURE_LIMIT: usize = 4 * 1024 * 1024;

pub fn shell_command(cmd: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    }
    #[cfg(not(windows))]
    {
        let shell = if Path::new("/bin/bash").exists() {
            "/bin/bash"
        } else {
            "/bin/sh"
        };
        let mut c = tokio::process::Command::new(shell);
        c.arg("-c").arg(cmd);
        c
    }
}

pub async fn run_shell(
    cmd: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
) -> anyhow::Result<ProcessResult> {
    let mut command = shell_command(cmd);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("TERM", "dumb")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("VEYRA", "1")
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);

    let start = Instant::now();
    let mut child = command.spawn()?;
    let pid = child.id();
    let mut out = child.stdout.take().expect("piped stdout");
    let mut err = child.stderr.take().expect("piped stderr");

    let read_out = tokio::spawn(async move { read_capped(&mut out).await });
    let read_err = tokio::spawn(async move { read_capped(&mut err).await });

    let mut timed_out = false;
    let mut cancelled = false;
    let status = tokio::select! {
        s = child.wait() => Some(s?),
        _ = tokio::time::sleep(timeout) => { timed_out = true; None }
        _ = cancel.cancelled() => { cancelled = true; None }
    };
    let status = match status {
        Some(s) => Some(s),
        None => {
            kill_tree(pid);
            let _ = child.kill().await;
            child.wait().await.ok()
        }
    };
    let stdout = read_out.await.unwrap_or_default();
    let stderr = read_err.await.unwrap_or_default();
    Ok(ProcessResult {
        command: cmd.to_string(),
        exit_code: if timed_out || cancelled {
            None
        } else {
            status.and_then(|s| s.code())
        },
        duration_ms: start.elapsed().as_millis() as u64,
        stdout,
        stderr,
        timed_out,
        cancelled,
    })
}

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut dropped = 0usize;
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if buf.len() < CAPTURE_LIMIT {
                    let take = n.min(CAPTURE_LIMIT - buf.len());
                    buf.extend_from_slice(&chunk[..take]);
                    dropped += n - take;
                } else {
                    dropped += n;
                }
            }
        }
    }
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if dropped > 0 {
        s.push_str(&format!("\n… [{dropped} bytes of output discarded] …\n"));
    }
    s
}

fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    #[cfg(unix)]
    unsafe {
        // Negative pid: signal the whole process group created above.
        libc::kill(-(pid as i32), libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(200));
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_exit_code_and_streams() {
        let d = tempfile::tempdir().unwrap();
        let r = run_shell("echo out; echo err 1>&2; exit 3", d.path(), Duration::from_secs(10), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(r.exit_code, Some(3));
        assert_eq!(r.stdout.trim(), "out");
        assert_eq!(r.stderr.trim(), "err");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_process_group() {
        let d = tempfile::tempdir().unwrap();
        let r = run_shell("sleep 30 & sleep 30; echo never", d.path(), Duration::from_millis(300), &CancellationToken::new())
            .await
            .unwrap();
        assert!(r.timed_out);
        assert_eq!(r.exit_code, None);
        assert!(r.duration_ms < 5000);
    }

    #[tokio::test]
    async fn cancellation() {
        let d = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        let t2 = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            t2.cancel();
        });
        #[cfg(unix)]
        let cmd = "sleep 30";
        #[cfg(windows)]
        let cmd = "ping -n 30 127.0.0.1";
        let r = run_shell(cmd, d.path(), Duration::from_secs(60), &token).await.unwrap();
        assert!(r.cancelled);
    }
}
