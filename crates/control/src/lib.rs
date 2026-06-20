//! Control API over a unix-domain socket (SPEC.md §6.2). This is the
//! human-facing interface — driven by the CLI's management commands and the
//! optional GUI — kept entirely separate from the agent-facing MCP server.
//!
//! Same JSON-RPC framing as MCP, different (dotted) method namespace. The
//! socket is bound 0600 under the user's data dir, so trust is scoped by
//! filesystem permissions and it is never reachable over the network.
//!
//! Run by `mailagent serve`, kept alive by a launchd login-item (daemon model
//! B), so the binary is the persistent thing and the GUI is a transient client.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use mailagent_core::MailAgent;
use mailagent_types::Permissions;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub async fn run_uds(agent: Arc<MailAgent>, socket_path: &Path) -> anyhow::Result<()> {
    // A stale socket file from a previous run would block bind().
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    let listener = UnixListener::bind(socket_path)?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
    eprintln!("control API listening on {}", socket_path.display());

    loop {
        let (stream, _addr) = listener.accept().await?;
        let agent = Arc::clone(&agent);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(agent, stream).await {
                eprintln!("control connection error: {e}");
            }
        });
    }
}

async fn handle_connection(agent: Arc<MailAgent>, stream: UnixStream) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params");

        let outcome = dispatch(&agent, method, params).await;

        let Some(id) = id else { continue }; // notifications get no response

        let response = match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": e.to_string() }
            }),
        };

        let mut serialized = serde_json::to_string(&response)?;
        serialized.push('\n');
        write_half.write_all(serialized.as_bytes()).await?;
        write_half.flush().await?;
    }

    Ok(())
}

async fn dispatch(agent: &MailAgent, method: &str, params: Option<&Value>) -> anyhow::Result<Value> {
    match method {
        "ping" => Ok(json!({ "ok": true })),

        "status" => Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "accounts": agent.list_accounts()?.len(),
            "pendingConfirmations": agent.pending_confirmations()?.len(),
        })),

        "accounts.list" => Ok(json!({ "accounts": agent.list_accounts()? })),

        "accounts.set_permissions" => {
            let params = params.ok_or_else(|| anyhow::anyhow!("missing params"))?;
            let account_id = params
                .get("accountId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("accountId is required"))?;
            let permissions: Permissions = serde_json::from_value(
                params
                    .get("permissions")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("permissions is required"))?,
            )?;
            let account = agent.set_account_permissions(account_id, permissions)?;
            Ok(json!({ "account": account }))
        }

        "audit.list" => {
            let limit = params
                .and_then(|p| p.get("limit"))
                .and_then(Value::as_u64)
                .unwrap_or(50) as u32;
            Ok(json!({ "events": agent.recent_audit(limit)? }))
        }

        "confirmations.list" => Ok(json!({ "confirmations": agent.pending_confirmations()? })),

        "confirmations.resolve" => {
            let params = params.ok_or_else(|| anyhow::anyhow!("missing params"))?;
            let id = params
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("id is required"))?;
            let approve = params.get("approve").and_then(Value::as_bool).unwrap_or(false);
            Ok(json!({ "resolved": agent.resolve_confirmation(id, approve)? }))
        }

        other => anyhow::bail!("unknown control method: {other}"),
    }
}
