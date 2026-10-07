//! Bridge between the desktop window and `kara serve --stdio`.
//!
//! The desktop app holds no agent logic of its own: it spawns the same
//! `kara` binary the CLI and the VS Code extension use, in server mode, and
//! speaks the same newline-delimited JSON-RPC (`kara_protocol::jsonrpc`)
//! over its stdio. This file is the Rust sibling of the VS Code extension's
//! `rpc.ts` + `agent.ts`.

use kara_protocol::jsonrpc::{methods, Message, RpcError};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

/// One running `kara serve --stdio` child and the plumbing to talk to it.
pub struct Session {
    child: AsyncMutex<Option<Child>>,
    stdin: AsyncMutex<Option<ChildStdin>>,
    pending: PendingMap,
    next_id: Mutex<u64>,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            child: AsyncMutex::new(None),
            stdin: AsyncMutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Mutex::new(1),
        }
    }
}

/// Find the `kara` binary: next to this app bundle first (for a packaged
/// `.dmg`/`.exe` that ships its own copy), else on `PATH` (a dev build or a
/// machine that already has the CLI installed, e.g. via Homebrew/winget).
fn locate_kara_binary(app: &AppHandle) -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "kara.exe" } else { "kara" };
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir.join(name);
        if bundled.is_file() {
            return Some(bundled);
        }
    }
    which(name)
}

/// Home directory without pulling in the `dirs` crate just for this.
fn dirs_home() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

/// Minimal `which`: search `PATH` ourselves rather than add a dependency.
fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(if cfg!(windows) { ';' } else { ':' })
        .map(std::path::PathBuf::from)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

impl Session {
    /// Start `kara serve --stdio` for `workspace` and begin relaying its
    /// output to the window as `kara://event` / `kara://log` events.
    pub async fn start(
        &self,
        app: AppHandle,
        workspace: String,
    ) -> Result<Value, String> {
        self.stop().await;
        // v1 has no folder picker yet: an empty workspace falls back to
        // the user's home directory rather than the app bundle's own cwd.
        let workspace = if workspace.trim().is_empty() {
            dirs_home().ok_or_else(|| "could not determine a home directory".to_string())?
        } else {
            workspace
        };
        let binary = locate_kara_binary(&app)
            .ok_or_else(|| "the `kara` binary was not found (bundled or on PATH)".to_string())?;
        let mut child = Command::new(&binary)
            .args(["serve", "--stdio"])
            .current_dir(&workspace)
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start Kara: {e}"))?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        *self.stdin.lock().await = Some(stdin);
        *self.child.lock().await = Some(child);

        let pending = self.pending.clone();
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                route_incoming(&app2, &pending, &line);
            }
            let _ = app2.emit("kara://exited", ());
        });
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit("kara://stderr", line);
            }
        });

        self.request(
            methods::INITIALIZE,
            serde_json::json!({ "workspace": workspace, "clientName": "desktop" }),
        )
        .await
        .map_err(|e| e.message)
    }

    /// Send a client->server request and wait for its response.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            let id = *n;
            *n += 1;
            id
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.write(&Message::request(id, method, params)).await?;
        rx.await.unwrap_or_else(|_| {
            Err(RpcError::new(-32000, "Kara exited before responding"))
        })
    }

    /// Send a one-way notification (no response expected), e.g. cancel.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), RpcError> {
        self.write(&Message::notification(method, params)).await
    }

    /// Answer a server->client request (currently only `permission/request`).
    pub async fn respond(&self, id: Value, result: Result<Value, RpcError>) -> Result<(), RpcError> {
        self.write(&Message::response(id, result)).await
    }

    async fn write(&self, msg: &Message) -> Result<(), RpcError> {
        let line = msg.encode_line();
        let mut guard = self.stdin.lock().await;
        let Some(stdin) = guard.as_mut() else {
            return Err(RpcError::new(-32000, "Kara is not running"));
        };
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| RpcError::new(-32000, format!("write to kara failed: {e}")))?;
        let _ = stdin.flush().await;
        Ok(())
    }

    pub async fn stop(&self) {
        *self.stdin.lock().await = None;
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.send(Err(RpcError::new(-32000, "Kara stopped")));
        }
    }
}

/// One decoded line from `kara`'s stdout: resolve a pending request, or
/// forward a notification/server-request to the window as a Tauri event.
fn route_incoming(app: &AppHandle, pending: &PendingMap, line: &str) {
    let msg = match Message::decode(line) {
        Ok(m) => m,
        Err(e) => {
            let _ = app.emit("kara://protocol-error", format!("{}: {line}", e.message));
            return;
        }
    };
    match msg {
        Message::Response { id, result } => {
            if let Some(id) = id.as_u64() {
                if let Some(tx) = pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(result);
                    return;
                }
            }
        }
        Message::Notification { method, params } => {
            let _ = app.emit(&format!("kara://{method}"), params);
        }
        Message::Request { id, method, params } => {
            // Only `permission/request` is expected from the server; the
            // window answers it via the `kara_respond` command, keyed by id.
            let _ = app.emit(
                "kara://request",
                serde_json::json!({ "id": id, "method": method, "params": params }),
            );
        }
    }
}
