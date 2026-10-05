//! `llama-server` child-process lifecycle.
//!
//! The server binds to a loopback address on a free port, its output goes to
//! `logs/local-runtime.log` in Kara's data directory, Kara waits for `/health`, and the
//! process is terminated when the handle is dropped or `stop` is called.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub binary: PathBuf,
    pub model_path: PathBuf,
    pub host: String,
    pub context: u32,
    /// -1 = offload all layers.
    pub gpu_layers: i32,
    /// Keep MoE expert weights in system RAM (`--cpu-moe`).
    pub cpu_moe: bool,
    pub alias: String,
    /// "auto", "on" or "off".
    pub reasoning: String,
    /// Thinking-token budget (-1 = unlimited).
    pub reasoning_budget: i32,
    pub extra_args: Vec<String>,
    pub log_file: PathBuf,
    pub startup_timeout: Duration,
}

pub struct LlamaServer {
    child: Option<tokio::process::Child>,
    pub port: u16,
    pub base_url: String,
    pub log_file: PathBuf,
    pub args: Vec<String>,
    pid_file: Option<PathBuf>,
}

/// Servers started by this process, so signal handlers can stop them.
static RUNNING: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

fn register(pid: u32) {
    RUNNING.lock().unwrap_or_else(|p| p.into_inner()).push(pid);
}

fn unregister(pid: u32) {
    RUNNING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|p| *p != pid);
}

/// Kill every server this process started (called from signal handlers).
pub fn kill_all_started() {
    let pids: Vec<u32> = RUNNING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .drain(..)
        .collect();
    for pid in pids {
        kill_pid(pid);
    }
}

fn kill_pid(pid: u32) {
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    if let Some(proc_) = sys.process(p) {
        proc_.kill();
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PidRecord {
    server_pid: u32,
    owner_pid: u32,
    binary: PathBuf,
}

/// Stop servers left behind by Kara processes that died without cleaning
/// up (e.g. `kill -9`). Servers whose owner is still running are untouched.
/// Returns the number of servers stopped.
pub fn reap_stale(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut reaped = 0;
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("pid") {
            continue;
        }
        let Ok(rec) = std::fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|b| Ok(serde_json::from_slice::<PidRecord>(&b)?))
        else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if sys.process(sysinfo::Pid::from_u32(rec.owner_pid)).is_some() {
            continue;
        }
        if let Some(p) = sys.process(sysinfo::Pid::from_u32(rec.server_pid)) {
            let same_binary = p.exe().map(|e| e == rec.binary).unwrap_or(false)
                || p.name().to_string_lossy().contains("llama-server");
            if same_binary {
                p.kill();
                reaped += 1;
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    reaped
}

pub fn free_port(host: &str) -> std::io::Result<u16> {
    let l = TcpListener::bind((host, 0))?;
    Ok(l.local_addr()?.port())
}

pub fn build_args(opts: &ServerOptions, port: u16) -> Vec<String> {
    let mut args = vec![
        "--model".into(),
        opts.model_path.display().to_string(),
        "--host".into(),
        opts.host.clone(),
        "--port".into(),
        port.to_string(),
        "--ctx-size".into(),
        opts.context.to_string(),
        "--alias".into(),
        opts.alias.clone(),
        "--jinja".into(),
        "--no-webui".into(),
        "--n-gpu-layers".into(),
        if opts.gpu_layers < 0 {
            "999".into()
        } else {
            opts.gpu_layers.to_string()
        },
    ];
    if opts.cpu_moe {
        args.push("--cpu-moe".into());
    }
    if matches!(opts.reasoning.as_str(), "on" | "off") {
        args.push("--reasoning".into());
        args.push(opts.reasoning.clone());
    }
    if opts.reasoning_budget >= 0 {
        args.push("--reasoning-budget".into());
        args.push(opts.reasoning_budget.to_string());
    }
    args.extend(opts.extra_args.iter().cloned());
    args
}

impl LlamaServer {
    pub async fn start(opts: ServerOptions) -> anyhow::Result<LlamaServer> {
        if !opts.model_path.is_file() {
            anyhow::bail!("model file not found: {}", opts.model_path.display());
        }
        let port = free_port(&opts.host)?;
        let args = build_args(&opts, port);
        if let Some(parent) = opts.log_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let log = std::fs::File::create(&opts.log_file)?;
        let mut cmd = tokio::process::Command::new(&opts.binary);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true);
        // Shared libraries ship next to the binary in llama.cpp release archives.
        if let Some(dir) = opts.binary.parent() {
            #[cfg(target_os = "linux")]
            cmd.env("LD_LIBRARY_PATH", prepend_env("LD_LIBRARY_PATH", dir));
            #[cfg(target_os = "macos")]
            cmd.env("DYLD_LIBRARY_PATH", prepend_env("DYLD_LIBRARY_PATH", dir));
            #[cfg(windows)]
            cmd.env("PATH", prepend_env("PATH", dir));
        }
        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("failed to start {}: {e}", opts.binary.display()))?;
        let pid_file = child.id().and_then(|pid| {
            register(pid);
            let dir = opts.log_file.parent()?;
            let rec = PidRecord {
                server_pid: pid,
                owner_pid: std::process::id(),
                binary: opts.binary.clone(),
            };
            let f = dir.join(format!("llama-server-{pid}.pid"));
            std::fs::write(&f, serde_json::to_vec(&rec).ok()?).ok()?;
            Some(f)
        });
        let host = if opts.host.contains(':') {
            format!("[{}]", opts.host)
        } else {
            opts.host.clone()
        };
        let mut server = LlamaServer {
            child: Some(child),
            port,
            base_url: format!("http://{host}:{port}/v1"),
            log_file: opts.log_file.clone(),
            args,
            pid_file,
        };
        server.wait_healthy(&host, opts.startup_timeout).await?;
        Ok(server)
    }

    async fn wait_healthy(&mut self, host: &str, timeout: Duration) -> anyhow::Result<()> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        let url = format!("http://{host}:{}/health", self.port);
        let start = Instant::now();
        loop {
            if let Some(child) = self.child.as_mut() {
                if let Some(status) = child.try_wait()? {
                    anyhow::bail!(
                        "llama-server exited during startup ({status}). Last log lines:\n{}",
                        tail(&self.log_file, 15)
                    );
                }
            }
            if let Ok(r) = client.get(&url).send().await {
                if r.status().is_success() {
                    return Ok(());
                }
            }
            if start.elapsed() > timeout {
                self.stop().await;
                anyhow::bail!(
                    "llama-server did not become healthy within {}s. Last log lines:\n{}",
                    timeout.as_secs(),
                    tail(&self.log_file, 15)
                );
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.as_mut().map(|c| c.try_wait()), Some(Ok(None)))
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.id())
    }

    /// Graceful stop: SIGTERM, then kill after a grace period.
    fn forget(&mut self, pid: Option<u32>) {
        if let Some(pid) = pid {
            unregister(pid);
        }
        if let Some(f) = self.pid_file.take() {
            let _ = std::fs::remove_file(f);
        }
    }

    pub async fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let pid = child.id();
        self.forget(pid);
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            unsafe_term(pid);
            if tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_ok()
            {
                return;
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            let _ = child.start_kill();
            self.forget(pid);
        }
    }
}

#[cfg(unix)]
fn unsafe_term(pid: u32) {
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn prepend_env(var: &str, dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_path_buf()];
    if let Some(old) = std::env::var_os(var) {
        paths.extend(std::env::split_paths(&old));
    }
    std::env::join_paths(paths).unwrap_or_default()
}

pub fn tail(path: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ServerOptions {
        ServerOptions {
            binary: "llama-server".into(),
            model_path: "/models/m.gguf".into(),
            host: "127.0.0.1".into(),
            context: 32768,
            gpu_layers: -1,
            cpu_moe: false,
            alias: "kara".into(),
            reasoning: "auto".into(),
            reasoning_budget: -1,
            extra_args: vec![],
            log_file: "/tmp/l.log".into(),
            startup_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn binds_loopback_only_by_default() {
        let args = build_args(&opts(), 5555);
        let host_idx = args.iter().position(|a| a == "--host").unwrap();
        assert_eq!(args[host_idx + 1], "127.0.0.1");
        assert!(!args.iter().any(|a| a == "0.0.0.0"));
        assert!(args.contains(&"--no-webui".to_string()));
        assert!(args.contains(&"--jinja".to_string()));
    }

    #[test]
    fn reasoning_flags() {
        let mut o = opts();
        assert!(!build_args(&o, 1).contains(&"--reasoning-budget".to_string()));
        o.reasoning_budget = 1024;
        o.reasoning = "off".into();
        let a = build_args(&o, 1);
        let i = a.iter().position(|x| x == "--reasoning-budget").unwrap();
        assert_eq!(a[i + 1], "1024");
        assert!(a.windows(2).any(|w| w[0] == "--reasoning" && w[1] == "off"));
    }

    #[test]
    fn moe_offload_flag() {
        let mut o = opts();
        o.cpu_moe = true;
        assert!(build_args(&o, 1).contains(&"--cpu-moe".to_string()));
    }

    #[test]
    fn free_port_is_loopback() {
        let p = free_port("127.0.0.1").unwrap();
        assert!(p > 0);
    }

    #[cfg(unix)]
    #[test]
    fn stale_servers_with_dead_owners_are_reaped() {
        let dir = tempfile::tempdir().unwrap();
        // A long-running stand-in for an orphaned llama-server.
        let mut orphan = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        // Owner pid that cannot exist.
        let rec = PidRecord {
            server_pid: orphan.id(),
            owner_pid: 999_999_999,
            binary: "/bin/sleep".into(),
        };
        std::fs::write(
            dir.path().join("llama-server-1.pid"),
            serde_json::to_vec(&rec).unwrap(),
        )
        .unwrap();
        // A live owner (this test process) must be left alone.
        let mut kept = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let rec2 = PidRecord {
            server_pid: kept.id(),
            owner_pid: std::process::id(),
            binary: "/bin/sleep".into(),
        };
        std::fs::write(
            dir.path().join("llama-server-2.pid"),
            serde_json::to_vec(&rec2).unwrap(),
        )
        .unwrap();

        assert_eq!(reap_stale(dir.path()), 1);
        assert!(orphan.wait().is_ok());
        assert!(
            kept.try_wait().unwrap().is_none(),
            "server of a live owner keeps running"
        );
        assert!(dir.path().join("llama-server-2.pid").exists());
        let _ = kept.kill();
    }

    #[tokio::test]
    async fn missing_model_is_reported() {
        let err = LlamaServer::start(opts()).await.err().unwrap();
        assert!(err.to_string().contains("model file not found"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn early_exit_is_reported_with_log() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake-server");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho 'error: failed to load model' >&2\nexit 3\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let model = dir.path().join("m.gguf");
        std::fs::write(&model, "gguf").unwrap();
        let mut o = opts();
        o.binary = fake;
        o.model_path = model;
        o.log_file = dir.path().join("server.log");
        o.startup_timeout = Duration::from_secs(10);
        let err = LlamaServer::start(o).await.err().unwrap().to_string();
        assert!(err.contains("exited during startup"), "{err}");
        assert!(err.contains("failed to load model"), "{err}");
    }
}
