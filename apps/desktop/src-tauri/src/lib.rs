mod cli;
mod rpc;
mod terminal;

use kara_protocol::jsonrpc::RpcError;
use rpc::Session;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};
use terminal::Terminal;

struct AppState {
    session: Session,
    terminal: Arc<Terminal>,
}

fn to_err(e: RpcError) -> String {
    e.message
}

#[tauri::command]
async fn kara_start(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace: String,
) -> Result<Value, String> {
    state.session.start(app, workspace).await
}

#[tauri::command]
async fn kara_request(
    state: State<'_, AppState>,
    method: String,
    params: Value,
) -> Result<Value, String> {
    state.session.request(&method, params).await.map_err(to_err)
}

#[tauri::command]
async fn kara_notify(
    state: State<'_, AppState>,
    method: String,
    params: Value,
) -> Result<(), String> {
    state.session.notify(&method, params).await.map_err(to_err)
}

/// Answer a server->client request (currently only `permission/request`).
#[tauri::command]
async fn kara_respond(
    state: State<'_, AppState>,
    id: Value,
    decision: Option<Value>,
    error: Option<String>,
) -> Result<(), String> {
    let result = match error {
        Some(msg) => Err(RpcError::new(-32000, msg)),
        None => Ok(decision.unwrap_or(Value::Null)),
    };
    state.session.respond(id, result).await.map_err(to_err)
}

#[tauri::command]
async fn kara_stop(state: State<'_, AppState>) -> Result<(), String> {
    state.session.stop().await;
    Ok(())
}

/// The folder-picker dialog's starting location only — never used to
/// start a session without the user actually choosing a folder.
#[tauri::command]
fn kara_home_dir() -> Option<String> {
    rpc::dirs_home()
}

#[tauri::command]
async fn kara_connect(app: AppHandle, url: String, token: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || cli::connect(&app, &url, &token))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn kara_config_set(app: AppHandle, key: String, value: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || cli::config_set(&app, &key, &value))
        .await
        .map_err(|e| e.to_string())?
}

/// For drag-and-drop: the window gets dropped paths but can't stat them.
#[tauri::command]
fn path_is_dir(path: String) -> bool {
    std::path::Path::new(&path).is_dir()
}

#[tauri::command]
fn terminal_spawn(app: AppHandle, state: State<'_, AppState>, cwd: String) -> Result<u64, String> {
    state.terminal.spawn(app, cwd)
}

#[tauri::command]
fn terminal_kill(state: State<'_, AppState>) {
    state.terminal.kill();
}

#[tauri::command]
fn terminal_write(state: State<'_, AppState>, data: String) -> Result<(), String> {
    state.terminal.write(&data)
}

#[tauri::command]
fn terminal_resize(state: State<'_, AppState>, rows: u16, cols: u16) -> Result<(), String> {
    state.terminal.resize(rows, cols)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(AppState {
            session: Session::default(),
            terminal: Arc::new(Terminal::default()),
        })
        .invoke_handler(tauri::generate_handler![
            kara_start,
            kara_request,
            kara_notify,
            kara_respond,
            kara_stop,
            kara_home_dir,
            path_is_dir,
            kara_connect,
            kara_config_set,
            terminal_spawn,
            terminal_kill,
            terminal_write,
            terminal_resize
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
