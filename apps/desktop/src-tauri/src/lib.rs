mod rpc;

use kara_protocol::jsonrpc::RpcError;
use rpc::Session;
use serde_json::Value;
use tauri::{AppHandle, State};

struct AppState {
    session: Session,
}

fn to_err(e: RpcError) -> String {
    e.message
}

#[tauri::command]
async fn kara_start(app: AppHandle, state: State<'_, AppState>, workspace: String) -> Result<Value, String> {
    state.session.start(app, workspace).await
}

#[tauri::command]
async fn kara_request(state: State<'_, AppState>, method: String, params: Value) -> Result<Value, String> {
    state.session.request(&method, params).await.map_err(to_err)
}

#[tauri::command]
async fn kara_notify(state: State<'_, AppState>, method: String, params: Value) -> Result<(), String> {
    state.session.notify(&method, params).await.map_err(to_err)
}

/// Answer a server->client request (currently only `permission/request`).
#[tauri::command]
async fn kara_respond(state: State<'_, AppState>, id: Value, decision: Option<Value>, error: Option<String>) -> Result<(), String> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(AppState {
            session: Session::default(),
        })
        .invoke_handler(tauri::generate_handler![
            kara_start,
            kara_request,
            kara_notify,
            kara_respond,
            kara_stop,
            kara_home_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
