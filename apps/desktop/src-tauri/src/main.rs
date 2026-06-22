//! Beeline desktop app (Tauri). A thin GUI over `mailagent-core`: the same
//! facade the CLI and MCP server use. Commands run on Tauri's async runtime and
//! delegate straight to core, so the GUI holds no business logic.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use mailagent_core::MailAgent;
use mailagent_types::{ConnectedAccount, Permissions};

#[tauri::command]
async fn get_accounts(agent: tauri::State<'_, MailAgent>) -> Result<Vec<ConnectedAccount>, String> {
    agent.list_accounts().map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_account(
    provider: String,
    alias: Option<String>,
    agent: tauri::State<'_, MailAgent>,
) -> Result<ConnectedAccount, String> {
    let result = match provider.as_str() {
        "gmail" => agent.add_gmail_account(alias).await,
        "microsoft" => agent.add_microsoft_account(alias).await,
        other => return Err(format!("unsupported provider: {other}")),
    };
    result.map_err(|e| e.to_string())
}

#[tauri::command]
async fn remove_account(account: String, agent: tauri::State<'_, MailAgent>) -> Result<(), String> {
    agent.remove_account(&account).map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_permissions(
    account_id: String,
    permissions: Permissions,
    agent: tauri::State<'_, MailAgent>,
) -> Result<ConnectedAccount, String> {
    agent
        .set_account_permissions(&account_id, permissions)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn connect_claude() -> Result<String, String> {
    let binary = resolve_mailagent_binary()?;
    let path = mailagent_core::install_mcp_client("claude", &binary).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Locate the `mailagent` helper the AI client should spawn. In a bundled app
/// it sits beside the GUI binary; in dev, point at it via BEELINE_MCP_BIN.
fn resolve_mailagent_binary() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("BEELINE_MCP_BIN") {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(sibling) = exe.parent().map(|d| d.join("mailagent")) {
        if sibling.exists() {
            return Ok(sibling);
        }
    }
    Err("mailagent helper not found — set BEELINE_MCP_BIN in dev, or bundle it beside the app".into())
}

fn main() {
    let db_path = mailagent_core::default_db_path().expect("resolve data dir");
    let agent = MailAgent::open(&db_path).expect("open mailagent store");

    tauri::Builder::default()
        .manage(agent)
        .invoke_handler(tauri::generate_handler![
            get_accounts,
            add_account,
            remove_account,
            set_permissions,
            connect_claude
        ])
        .run(tauri::generate_context!())
        .expect("error while running Beeline");
}
