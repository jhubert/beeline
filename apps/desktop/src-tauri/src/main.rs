//! Beeline desktop app (Tauri). A thin GUI over `mailagent-core`: the same
//! facade the CLI and MCP server use. Commands run on Tauri's async runtime and
//! delegate straight to core, so the GUI holds no business logic.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

fn main() {
    let db_path = mailagent_core::default_db_path().expect("resolve data dir");
    let agent = MailAgent::open(&db_path).expect("open mailagent store");

    tauri::Builder::default()
        .manage(agent)
        .invoke_handler(tauri::generate_handler![
            get_accounts,
            add_account,
            remove_account,
            set_permissions
        ])
        .run(tauri::generate_context!())
        .expect("error while running Beeline");
}
