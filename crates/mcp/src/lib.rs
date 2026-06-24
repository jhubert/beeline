//! Local MCP server over stdio (SPEC.md §6.2). MCP is JSON-RPC 2.0 over
//! newline-delimited messages on stdin/stdout — small enough to hand-roll, so
//! we don't take an SDK dependency. The client (Claude Desktop, Cursor, …)
//! spawns this process; trust is inherited from the process boundary, so there
//! is no network listener and no auth handshake here.
//!
//! Phase 0 exposes the read path: mail_list_accounts, mail_search,
//! mail_get_message. Mutating tools arrive with the policy/confirmation layer.

use std::sync::Arc;

use mailagent_core::MailAgent;
use mailagent_types::MailSearchQuery;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-06-18";

pub async fn run_stdio(agent: Arc<MailAgent>) -> anyhow::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue, // ignore malformed frames
        };

        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params");

        let outcome = dispatch(&agent, method, params).await;

        // Notifications (no id) get no response.
        let Some(id) = id else { continue };

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
        stdout.write_all(serialized.as_bytes()).await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn dispatch(agent: &MailAgent, method: &str, params: Option<&Value>) -> anyhow::Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "beeline", "version": env!("CARGO_PKG_VERSION") }
        })),
        "tools/list" => Ok(json!({ "tools": tool_schemas() })),
        "tools/call" => call_tool(agent, params).await,
        "ping" => Ok(json!({})),
        other => anyhow::bail!("method not found: {other}"),
    }
}

fn tool_schemas() -> Value {
    json!([
        {
            "name": "mail_list_accounts",
            "description": "List connected email accounts and their permissions.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "mail_search",
            "description": "Search messages across one or all connected accounts. Prefer the structured fields (from, to, subject, since, before) over provider-specific operators in `query` — they translate consistently across Gmail and Outlook.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": { "type": "string", "description": "Account alias or \"all\"." },
                    "query": { "type": "string", "description": "Free-text search." },
                    "from": { "type": "string", "description": "Sender to match." },
                    "to": { "type": "string", "description": "Recipient to match." },
                    "subject": { "type": "string" },
                    "since": { "type": "string", "description": "ISO date (YYYY-MM-DD) lower bound." },
                    "before": { "type": "string", "description": "ISO date (YYYY-MM-DD) upper bound." },
                    "unreadOnly": { "type": "boolean" },
                    "hasAttachments": { "type": "boolean" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                }
            }
        },
        {
            "name": "mail_get_message",
            "description": "Read a specific message by its localMessageId.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "localMessageId": { "type": "string" },
                    "includeHtml": { "type": "boolean" },
                    "includeAttachments": { "type": "boolean" }
                },
                "required": ["localMessageId"]
            }
        },
        {
            "name": "mail_create_draft",
            "description": "Create a new draft (the user reviews and sends it themselves).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "account": { "type": "string", "description": "Account alias." },
                    "to": { "type": "array", "items": { "type": "string" } },
                    "cc": { "type": "array", "items": { "type": "string" } },
                    "bcc": { "type": "array", "items": { "type": "string" } },
                    "subject": { "type": "string" },
                    "bodyText": { "type": "string" }
                },
                "required": ["account"]
            }
        },
        {
            "name": "mail_create_draft_reply",
            "description": "Create a draft reply to a message (the user reviews and sends it).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "localMessageId": { "type": "string" },
                    "replyAll": { "type": "boolean" },
                    "bodyText": { "type": "string" }
                },
                "required": ["localMessageId", "bodyText"]
            }
        }
    ])
}

async fn call_tool(agent: &MailAgent, params: Option<&Value>) -> anyhow::Result<Value> {
    let params = params.ok_or_else(|| anyhow::anyhow!("missing params"))?;
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let payload = match name {
        "mail_list_accounts" => json!({ "accounts": agent.list_accounts()? }),

        "mail_search" => {
            let selector = args
                .get("account")
                .and_then(Value::as_str)
                .unwrap_or("all")
                .to_string();
            let query = MailSearchQuery {
                free_text: str_arg(&args, "query"),
                from: str_arg(&args, "from"),
                to: str_arg(&args, "to"),
                subject: str_arg(&args, "subject"),
                since: str_arg(&args, "since"),
                before: str_arg(&args, "before"),
                unread_only: args.get("unreadOnly").and_then(Value::as_bool),
                has_attachments: args.get("hasAttachments").and_then(Value::as_bool),
                limit: args.get("limit").and_then(Value::as_u64).map(|n| n as u32),
            };
            serde_json::to_value(agent.search(&selector, &query).await?)?
        }

        "mail_get_message" => {
            let local_id = args
                .get("localMessageId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("localMessageId is required"))?;
            json!({ "message": agent.get_message(local_id).await? })
        }

        "mail_create_draft" => {
            let account = args
                .get("account")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("account is required"))?
                .to_string();
            let input: mailagent_types::DraftInput = serde_json::from_value(args.clone())
                .map_err(|e| anyhow::anyhow!("invalid draft input: {e}"))?;
            json!({ "draft": agent.create_draft(&account, input).await? })
        }

        "mail_create_draft_reply" => {
            let local_id = args
                .get("localMessageId")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("localMessageId is required"))?;
            let reply_all = args.get("replyAll").and_then(Value::as_bool).unwrap_or(false);
            let body = args.get("bodyText").and_then(Value::as_str).unwrap_or("");
            json!({ "draft": agent.create_draft_reply(local_id, reply_all, body).await? })
        }

        other => anyhow::bail!("unknown tool: {other}"),
    };

    // MCP tool results are returned as content blocks; we serialize the
    // structured payload as text so any client can render it.
    Ok(json!({
        "content": [
            { "type": "text", "text": serde_json::to_string_pretty(&payload)? }
        ]
    }))
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}
