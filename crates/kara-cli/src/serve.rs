//! `kara serve --stdio`: JSON-RPC 2.0 over stdin/stdout for editor clients.
//!
//! Framing: one JSON object per line. No network port is opened. All agent
//! logic stays here; the VS Code extension is only a UI.
//!
//! Nothing in this module may print to stdout except protocol messages.
//! Diagnostics go to stderr, which the extension shows in its output channel.

use crate::app::{App, Options};
use crate::models::{self, Consent, ModelRuntime};
use crate::tui::{no_model_message, NoModel};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;
use kara_agent::approver::Approver;
use kara_agent::Agent;
use kara_core::permissions::{PermissionPolicy, Profile};
use kara_model::hardware::HardwareInfo;
use kara_model::recommend::recommend;
use kara_model::ModelProvider;
use kara_protocol::jsonrpc::{codes, methods, Message, RpcError};
use kara_protocol::{AgentMode, PermissionDecision, PermissionRequest, PROTOCOL_VERSION};
use kara_runtime::store::ModelStore;

type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

#[derive(Clone)]
struct Peer {
    tx: mpsc::UnboundedSender<Message>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

impl Peer {
    fn notify(&self, method: &str, params: Value) {
        let _ = self.tx.send(Message::notification(method, params));
    }

    fn respond(&self, id: Value, result: Result<Value, RpcError>) {
        let _ = self.tx.send(Message::response(id, result));
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (otx, orx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, otx);
        let _ = self.tx.send(Message::request(id, method, params));
        orx.await
            .unwrap_or_else(|_| Err(RpcError::new(codes::INTERNAL_ERROR, "client disconnected")))
    }

    fn log(&self, level: &str, message: impl Into<String>) {
        self.notify(
            methods::LOG,
            json!({"level": level, "message": message.into()}),
        );
    }
}

/// Asks the editor to approve actions.
struct RpcApprover {
    peer: Peer,
}

#[async_trait::async_trait]
impl Approver for RpcApprover {
    async fn decide(&self, r: &PermissionRequest) -> PermissionDecision {
        let params = serde_json::to_value(r).unwrap_or_default();
        match self.peer.request(methods::PERMISSION, params).await {
            Ok(v) => {
                let d = v.get("decision").and_then(Value::as_str).unwrap_or("deny");
                match d {
                    "allow_once" => PermissionDecision::AllowOnce,
                    "allow_session" if r.can_remember => PermissionDecision::AllowSession,
                    "allow_session" => PermissionDecision::AllowOnce,
                    _ => PermissionDecision::Deny,
                }
            }
            Err(_) => PermissionDecision::Deny,
        }
    }
}

struct ServeSession {
    app: App,
    runtime: ModelRuntime,
    agent: Agent,
    session_id: String,
}

struct Server {
    peer: Peer,
    session: Arc<Mutex<Option<ServeSession>>>,
    cancel: Arc<std::sync::Mutex<Option<CancellationToken>>>,
    opts: Options,
}

fn invalid(msg: impl Into<String>) -> RpcError {
    RpcError::new(codes::INVALID_PARAMS, msg)
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(codes::INTERNAL_ERROR, format!("{e:#}"))
}

fn runtime_json(r: &ModelRuntime) -> Value {
    json!({
        "available": r.provider.is_some(),
        "label": r.label,
        "id": r.spec.as_ref().map(|s| s.id.clone()),
        "context": r.context,
    })
}

impl Server {
    async fn handle(self: Arc<Self>, id: Value, method: String, params: Value) {
        let result = match method.as_str() {
            methods::INITIALIZE => self.initialize(params).await,
            methods::PROMPT => {
                // Long-running: respond when the turn ends.
                let me = self.clone();
                tokio::spawn(async move {
                    let r = me.prompt(params).await;
                    me.peer.respond(id, r);
                });
                return;
            }
            methods::CANCEL => {
                if let Some(t) = self.cancel.lock().unwrap().as_ref() {
                    t.cancel();
                }
                Ok(json!({"cancelled": true}))
            }
            methods::STATUS => self.status().await,
            methods::CHANGES => self.changes().await,
            methods::UNDO => self.undo().await,
            methods::COMMAND => self.command(params).await,
            methods::NEW_SESSION => self.new_session().await,
            methods::DOCTOR => self.doctor().await,
            methods::MODELS => self.models().await,
            methods::SELECT_MODEL => {
                let me = self.clone();
                tokio::spawn(async move {
                    let r = me.select_model(params).await;
                    me.peer.respond(id, r);
                });
                return;
            }
            methods::SHUTDOWN => {
                if let Some(s) = self.session.lock().await.as_mut() {
                    s.runtime.shutdown().await;
                }
                self.peer.respond(id, Ok(json!(null)));
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                std::process::exit(0);
            }
            other => Err(RpcError::new(
                codes::METHOD_NOT_FOUND,
                format!("unknown method {other}"),
            )),
        };
        self.peer.respond(id, result);
    }

    async fn initialize(&self, params: Value) -> Result<Value, RpcError> {
        let mut opts = self.opts.clone();
        if let Some(ws) = params.get("workspace").and_then(Value::as_str) {
            opts.dir = Some(ws.into());
        }
        if let Some(p) = params.get("profile").and_then(Value::as_str) {
            opts.profile = Some(p.to_string());
        }
        let app = App::load(&opts).map_err(internal)?;
        for w in &app.warnings {
            self.peer.log("warning", w.clone());
        }
        let _ = app.refresh_index_in_background();
        // Never download without explicit consent from the editor UI.
        let runtime = match models::start_runtime(&app, Consent::Never, None).await {
            Ok(r) => r,
            Err(e) => {
                self.peer.log("warning", format!("{e:#}"));
                ModelRuntime::none(&format!("{e:#}"))
            }
        };
        let session_id = app
            .sessions
            .create(&app.root, &runtime.label)
            .map_err(internal)?;
        let agent = self.build_agent(&app, &runtime, &session_id)?;
        let reply = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "karaVersion": kara_core::VERSION,
            "workspace": app.root,
            "session": session_id,
            "profile": app.config.permissions.profile.as_str(),
            "model": runtime_json(&runtime),
            "commands": crate::commands::COMMANDS.iter().map(|(n, d)| json!({"name": n, "description": d})).collect::<Vec<_>>(),
        });
        let mut guard = self.session.lock().await;
        if let Some(old) = guard.as_mut() {
            old.runtime.shutdown().await;
        }
        *guard = Some(ServeSession {
            app,
            runtime,
            agent,
            session_id,
        });
        Ok(reply)
    }

    fn build_agent(
        &self,
        app: &App,
        runtime: &ModelRuntime,
        session_id: &str,
    ) -> Result<Agent, RpcError> {
        let ctx = app.tool_context(session_id).map_err(internal)?;
        let provider: Arc<dyn ModelProvider> = match &runtime.provider {
            Some(p) => p.clone(),
            None => Arc::new(NoModel(no_model_message())),
        };
        let peer = self.peer.clone();
        Ok(Agent::new(
            provider,
            ctx,
            PermissionPolicy::new(app.config.permissions.profile),
            Arc::new(RpcApprover {
                peer: self.peer.clone(),
            }),
            Arc::new(move |e| {
                peer.notify(methods::EVENT, serde_json::to_value(&e).unwrap_or_default())
            }),
            app.agent_settings(runtime.context),
        ))
    }

    async fn prompt(&self, params: Value) -> Result<Value, RpcError> {
        let text = params
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let mode = match params
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("execute")
        {
            "plan" => AgentMode::Plan,
            "review" => AgentMode::Review,
            _ => AgentMode::Execute,
        };
        let approve = params
            .get("approvePlan")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut task = text.clone();
        if let Some(ctx) = params.get("context") {
            task.push_str(&format_editor_context(ctx));
        }
        if task.is_empty() && mode != AgentMode::Review && !approve {
            return Err(invalid("`text` is required"));
        }
        let mut guard = self
            .session
            .try_lock()
            .map_err(|_| RpcError::new(codes::BUSY, "Kara is already working on a task"))?;
        let s = guard
            .as_mut()
            .ok_or_else(|| invalid("call initialize first"))?;
        if s.runtime.provider.is_none() {
            return Err(RpcError::new(codes::MODEL_UNAVAILABLE, no_model_message()));
        }
        let token = CancellationToken::new();
        s.agent.ctx.cancel = token.clone();
        *self.cancel.lock().unwrap() = Some(token);
        let started = chrono::Utc::now().to_rfc3339();
        let result = if approve {
            match s.agent.approve_plan().await {
                Some(r) => r,
                None => return Err(invalid("no plan is awaiting approval")),
            }
        } else {
            let task = if task.is_empty() {
                "Review my current diff.".to_string()
            } else {
                task
            };
            s.agent.run_turn(&task, mode).await
        };
        *self.cancel.lock().unwrap() = None;
        let rec = kara_agent::session::TurnRecord {
            turn: result.turn_id,
            task: text.chars().take(2000).collect(),
            mode: format!("{:?}", result.mode).to_lowercase(),
            outcome: format!("{:?}", result.outcome).to_lowercase(),
            summary: result.summary.chars().take(4000).collect(),
            changed_files: result.changed_files.clone(),
            prompt_tokens: result.stats.usage.prompt_tokens,
            completion_tokens: result.stats.usage.completion_tokens,
            started,
            finished: chrono::Utc::now().to_rfc3339(),
        };
        let _ = s.app.sessions.record_turn(&s.session_id, &rec);
        let (state, history) = s.agent.snapshot();
        let _ = s.app.sessions.save_state(&s.session_id, &state, &history);
        serde_json::to_value(&result).map_err(internal)
    }

    async fn status(&self) -> Result<Value, RpcError> {
        let guard = self
            .session
            .try_lock()
            .map_err(|_| RpcError::new(codes::BUSY, "busy"))?;
        let s = guard
            .as_ref()
            .ok_or_else(|| invalid("call initialize first"))?;
        let index = s.app.index.stats().ok();
        Ok(json!({
            "workspace": s.app.root,
            "session": s.session_id,
            "model": runtime_json(&s.runtime),
            "profile": s.agent.policy.profile.as_str(),
            "pendingPlan": s.agent.pending_plan().map(|(t, p)| json!({"task": t, "plan": p})),
            "task": s.agent.state.task,
            "plan": s.agent.state.plan,
            "filesRead": s.agent.state.files_read,
            "filesChanged": s.agent.state.files_changed,
            "context": {"historyTokens": s.agent.history_tokens(), "window": s.agent.settings.context_window},
            "usage": s.agent.total_usage,
            "index": index,
        }))
    }

    async fn changes(&self) -> Result<Value, RpcError> {
        let guard = self
            .session
            .try_lock()
            .map_err(|_| RpcError::new(codes::BUSY, "busy"))?;
        let s = guard
            .as_ref()
            .ok_or_else(|| invalid("call initialize first"))?;
        let j = s.agent.ctx.journal();
        let root = &s.app.root;
        let mut files = Vec::new();
        for path in j.kara_changed_paths() {
            let before_bytes = j.original_content(&path).flatten();
            let after_bytes = std::fs::read(root.join(&path)).ok();
            let binary = before_bytes
                .as_deref()
                .map(kara_context::looks_binary)
                .unwrap_or(false)
                || after_bytes
                    .as_deref()
                    .map(kara_context::looks_binary)
                    .unwrap_or(false);
            if binary {
                files.push(json!({"path": path, "binary": true, "before": null, "after": null, "diff": ""}));
                continue;
            }
            let before = before_bytes.map(|b| String::from_utf8_lossy(&b).into_owned());
            let after = after_bytes.map(|b| String::from_utf8_lossy(&b).into_owned());
            let diff = kara_tools::patch::unified_diff(
                &path,
                before.as_deref().unwrap_or(""),
                after.as_deref().unwrap_or(""),
            );
            files.push(json!({"path": path, "before": before, "after": after, "diff": diff}));
        }
        let user: Vec<String> = kara_context::git::dirty_paths(root)
            .into_iter()
            .filter(|p| !j.kara_changed_paths().contains(p))
            .collect();
        Ok(json!({"kara": files, "user": user}))
    }

    async fn undo(&self) -> Result<Value, RpcError> {
        let guard = self
            .session
            .try_lock()
            .map_err(|_| RpcError::new(codes::BUSY, "busy"))?;
        let s = guard
            .as_ref()
            .ok_or_else(|| invalid("call initialize first"))?;
        let report = s.agent.ctx.journal().undo(None).map_err(internal)?;
        serde_json::to_value(report).map_err(internal)
    }

    async fn new_session(&self) -> Result<Value, RpcError> {
        let mut guard = self
            .session
            .try_lock()
            .map_err(|_| RpcError::new(codes::BUSY, "busy"))?;
        let s = guard
            .as_mut()
            .ok_or_else(|| invalid("call initialize first"))?;
        let id = s
            .app
            .sessions
            .create(&s.app.root, &s.runtime.label)
            .map_err(internal)?;
        let agent = self.build_agent(&s.app, &s.runtime, &id)?;
        s.agent = agent;
        s.session_id = id.clone();
        Ok(json!({"session": id}))
    }

    async fn command(&self, params: Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();
        let arg = params
            .get("args")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut guard = self
            .session
            .try_lock()
            .map_err(|_| RpcError::new(codes::BUSY, "busy"))?;
        let s = guard
            .as_mut()
            .ok_or_else(|| invalid("call initialize first"))?;
        match name.as_str() {
            "clear" => {
                s.agent.clear();
                Ok(json!({"text": "Conversation cleared."}))
            }
            "compact" => {
                let (a, b) = s.agent.compact_history();
                Ok(json!({"text": format!("Conversation compacted: ~{a} -> ~{b} tokens.")}))
            }
            "permissions" => {
                if !arg.is_empty() {
                    let p = Profile::parse(&arg)
                        .ok_or_else(|| invalid("profile must be safe, balanced or autonomous"))?;
                    s.agent.policy.set_profile(p);
                }
                let rows: Vec<Value> = s
                    .agent
                    .policy
                    .table()
                    .into_iter()
                    .map(|(k, d, g)| json!({"kind": k.label(), "decision": format!("{d:?}").to_lowercase(), "sessionGrant": g, "hardBoundary": k.is_hard_boundary()}))
                    .collect();
                Ok(json!({"profile": s.agent.policy.profile.as_str(), "rows": rows}))
            }
            "revert" => {
                s.agent.ctx.journal().revert_path(&arg).map_err(internal)?;
                Ok(json!({"reverted": arg}))
            }
            "privacy" => Ok(
                serde_json::to_value(kara_core::privacy::PrivacyReport::from_config(
                    &s.app.config,
                ))
                .map_err(internal)?,
            ),
            "matrix" => {
                let stats = s.app.index.stats().ok();
                let tools: Vec<&str> = s.agent.tools().iter().map(|t| t.name()).collect();
                Ok(json!({
                    "repository": stats,
                    "filesRead": s.agent.state.files_read,
                    "filesChanged": s.agent.state.files_changed,
                    "model": runtime_json(&s.runtime),
                    "context": {"historyTokens": s.agent.history_tokens(), "window": s.agent.settings.context_window},
                    "tools": tools,
                    "task": s.agent.state.task,
                    "plan": s.agent.state.plan,
                    "state": if s.agent.pending_plan().is_some() { "awaiting plan approval" } else { "idle" },
                }))
            }
            other => Err(invalid(format!(
                "unsupported command `{other}` over RPC; send natural language via session/prompt"
            ))),
        }
    }

    async fn doctor(&self) -> Result<Value, RpcError> {
        let paths = kara_core::KaraPaths::discover().map_err(internal)?;
        let models_dir = paths.models_dir();
        let hw = tokio::task::spawn_blocking(move || HardwareInfo::detect(&models_dir))
            .await
            .map_err(internal)?;
        let registry =
            kara_model::registry::Registry::load(&paths.user_models_file()).map_err(internal)?;
        let rec = recommend(&registry, &hw);
        let mgr = kara_runtime::llamacpp::LlamaCppManager::new(&paths.runtimes_dir());
        Ok(json!({
            "hardware": hw,
            "budget": hw.fast_memory_budget(),
            "recommendation": rec,
            "llamaCpp": mgr.locate("").map(|l| l.describe()),
        }))
    }

    async fn models(&self) -> Result<Value, RpcError> {
        let paths = kara_core::KaraPaths::discover().map_err(internal)?;
        let models_dir = paths.models_dir();
        let hw = tokio::task::spawn_blocking({
            let d = models_dir.clone();
            move || HardwareInfo::detect(&d)
        })
        .await
        .map_err(internal)?;
        let registry =
            kara_model::registry::Registry::load(&paths.user_models_file()).map_err(internal)?;
        let rec = recommend(&registry, &hw);
        let store = ModelStore::new(&models_dir);
        let list: Vec<Value> = registry
            .models
            .iter()
            .map(|m| {
                let c = rec.candidates.iter().find(|c| c.id == m.id);
                json!({
                    "id": m.id, "name": m.name, "quantization": m.quantization, "sizeBytes": m.size_bytes,
                    "license": m.license, "source": m.source_url(), "revision": m.revision,
                    "installed": store.is_installed(m), "fits": c.map(|c| c.fits), "reason": c.map(|c| c.reason.clone()),
                    "memoryNeeded": c.map(|c| c.memory_needed), "recommended": rec.model.as_ref().map(|r| r.id == m.id).unwrap_or(false),
                })
            })
            .collect();
        Ok(json!({"models": list, "summary": rec.summary}))
    }

    async fn select_model(&self, params: Value) -> Result<Value, RpcError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_string();
        let download = params
            .get("download")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut guard = self.session.lock().await;
        let s = guard
            .as_mut()
            .ok_or_else(|| invalid("call initialize first"))?;
        s.runtime.shutdown().await;
        self.peer.log(
            "info",
            format!(
                "starting model {id}{}",
                if download {
                    " (downloading if needed)"
                } else {
                    ""
                }
            ),
        );
        let consent = if download {
            Consent::Granted
        } else {
            Consent::Never
        };
        let runtime = models::start_runtime(&s.app, consent, Some(&id))
            .await
            .map_err(|e| RpcError::new(codes::MODEL_UNAVAILABLE, format!("{e:#}")))?;
        let provider: Arc<dyn ModelProvider> = match &runtime.provider {
            Some(p) => p.clone(),
            None => Arc::new(NoModel(no_model_message())),
        };
        s.agent.set_provider(provider);
        let reply = runtime_json(&runtime);
        s.runtime = runtime;
        Ok(reply)
    }
}

/// Render editor context (selection, file, diagnostics) into the task.
fn format_editor_context(ctx: &Value) -> String {
    let mut s = String::new();
    if let Some(file) = ctx.get("file").and_then(Value::as_str) {
        s.push_str(&format!("\n\nCurrent file in the editor: {file}"));
    }
    if let Some(sel) = ctx.get("selection") {
        if let Some(text) = sel
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
        {
            let start = sel.get("startLine").and_then(Value::as_u64).unwrap_or(0);
            let end = sel.get("endLine").and_then(Value::as_u64).unwrap_or(0);
            let clipped: String = text.chars().take(12_000).collect();
            s.push_str(&format!(
                "\nSelected code (lines {start}-{end}):\n```\n{clipped}\n```"
            ));
        }
    }
    if let Some(diags) = ctx.get("diagnostics").and_then(Value::as_array) {
        if !diags.is_empty() {
            s.push_str("\nEditor diagnostics:\n");
            for d in diags.iter().take(30) {
                s.push_str(&format!(
                    "- {}:{}: {}: {}\n",
                    d.get("file").and_then(Value::as_str).unwrap_or(""),
                    d.get("line").and_then(Value::as_u64).unwrap_or(0),
                    d.get("severity").and_then(Value::as_str).unwrap_or("error"),
                    d.get("message").and_then(Value::as_str).unwrap_or("")
                ));
            }
        }
    }
    s
}

pub fn serve_stdio(rt: &tokio::runtime::Runtime, opts: &Options) -> anyhow::Result<i32> {
    rt.block_on(async {
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let writer = tokio::spawn(async move {
            let mut out = tokio::io::stdout();
            while let Some(m) = rx.recv().await {
                if out.write_all(m.encode_line().as_bytes()).await.is_err() {
                    break;
                }
                let _ = out.flush().await;
            }
        });
        let peer = Peer {
            tx,
            pending: Arc::new(std::sync::Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        };
        let server = Arc::new(Server {
            peer: peer.clone(),
            session: Arc::new(Mutex::new(None)),
            cancel: Arc::new(std::sync::Mutex::new(None)),
            opts: opts.clone(),
        });
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            match Message::decode(&line) {
                Ok(Message::Request { id, method, params }) => {
                    let s = server.clone();
                    tokio::spawn(s.handle(id, method, params));
                }
                Ok(Message::Notification { method, .. }) => {
                    if method == methods::CANCEL {
                        if let Some(t) = server.cancel.lock().unwrap().as_ref() {
                            t.cancel();
                        }
                    }
                }
                Ok(Message::Response { id, result }) => {
                    if let Some(n) = id.as_u64() {
                        if let Some(tx) = peer.pending.lock().unwrap().remove(&n) {
                            let _ = tx.send(result);
                        }
                    }
                }
                Err(e) => peer.respond(Value::Null, Err(e)),
            }
        }
        // stdin closed: the editor went away. Stop the model runtime.
        if let Some(s) = server.session.lock().await.as_mut() {
            s.runtime.shutdown().await;
        }
        drop(peer);
        drop(server);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), writer).await;
        Ok(0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_context_is_rendered() {
        let s = format_editor_context(&json!({
            "file": "src/a.ts",
            "selection": {"text": "const x = 1;", "startLine": 3, "endLine": 3},
            "diagnostics": [{"file": "src/a.ts", "line": 3, "severity": "error", "message": "x is unused"}]
        }));
        assert!(s.contains("src/a.ts"));
        assert!(s.contains("lines 3-3"));
        assert!(s.contains("x is unused"));
    }
}
