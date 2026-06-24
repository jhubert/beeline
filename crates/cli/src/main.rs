//! `mailagent` — the distributable binary (SPEC.md §6.3). This single binary is
//! the product: it is the CLI, the MCP server (`mailagent mcp`), and will host
//! the control-API daemon (`mailagent serve`) that the optional GUI drives.

use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use mailagent_core::MailAgent;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Parser)]
#[command(name = "mailagent", version, about = "Local AI mail agent")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run local self-checks.
    Doctor,
    /// List connected accounts.
    Accounts,
    /// Connect an email account via OAuth (browser sign-in).
    AddAccount {
        /// Provider: gmail or microsoft.
        #[arg(long, default_value = "gmail")]
        provider: String,
        /// Optional alias; defaults to the local-part of the email.
        #[arg(long)]
        alias: Option<String>,
    },
    /// Search mail across accounts.
    Search {
        /// Free-text query (provider search syntax supported, e.g. "from:bruce").
        query: String,
        /// Account alias, or "all".
        #[arg(long, default_value = "all")]
        account: String,
        /// Max results.
        #[arg(long)]
        limit: Option<u32>,
        /// Only unread messages.
        #[arg(long)]
        unread: bool,
    },
    /// Read a message by its localMessageId (from search results).
    Read { local_message_id: String },
    /// Re-authorize an account whose token expired or was revoked.
    Reconnect { account: String },
    /// Enable/disable an account capability (currently: draft creation).
    Permissions {
        account: String,
        /// Allow creating drafts for this account (true/false).
        #[arg(long)]
        draft: Option<bool>,
    },
    /// Create a draft reply to a message (by localMessageId from search).
    DraftReply {
        local_message_id: String,
        body: String,
        #[arg(long)]
        reply_all: bool,
    },
    /// Disconnect an account (by alias or id) and delete its local token.
    RemoveAccount { account: String },
    /// Register Beeline's MCP server in an AI client's config (e.g. claude).
    InstallMcp {
        #[arg(default_value = "claude")]
        client: String,
    },
    /// Start the MCP server on stdio (launched by AI clients).
    Mcp,
    /// Run the control-API daemon for the GUI (launchd login-item, model B).
    Serve,
    /// Send a single request to the running control daemon (debug helper).
    Ctl {
        /// Control method, e.g. status, accounts.list, confirmations.list.
        method: String,
        /// JSON params object, e.g. '{"limit":10}'.
        #[arg(long)]
        params: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let db_path = mailagent_core::default_db_path()?;
    let agent = Arc::new(MailAgent::open(&db_path)?);

    match cli.command {
        Command::Doctor => {
            println!("mailagent doctor");
            println!("  [ok]   binary runs");
            println!("  [ok]   sqlite store at {}", db_path.display());
            println!("  [ok]   {} account(s) registered", agent.list_accounts()?.len());
            match mailagent_core::config::OAuthConfig::load() {
                Ok(c) => println!(
                    "  [ok]   OAuth client config present (gmail{})",
                    if c.microsoft_client_id.is_empty() { "" } else { " + microsoft" }
                ),
                Err(_) => println!(
                    "  [todo] OAuth client config missing — embed at build, or add ~/.mailagent/config.toml"
                ),
            }
        }
        Command::Accounts => {
            for a in agent.list_accounts()? {
                println!(
                    "{:<10} {:<22} {:<10?} {:?}",
                    a.alias, a.email, a.provider, a.status
                );
            }
        }
        Command::AddAccount { provider, alias } => {
            let account = match provider.as_str() {
                "gmail" | "google" => agent.add_gmail_account(alias).await?,
                "microsoft" | "outlook" => agent.add_microsoft_account(alias).await?,
                other => anyhow::bail!("unsupported provider '{other}' (try: gmail, microsoft)"),
            };
            println!(
                "Connected {} as \"{}\" (read-only).",
                account.email, account.alias
            );
        }
        Command::Search {
            query,
            account,
            limit,
            unread,
        } => {
            let q = mailagent_types::MailSearchQuery {
                free_text: (!query.is_empty()).then_some(query),
                unread_only: unread.then_some(true),
                limit,
                ..Default::default()
            };
            let found = agent.search(&account, &q).await?;
            if found.results.is_empty() {
                println!("(no results)");
            }
            for m in &found.results {
                println!(
                    "{}  [{}]  {}  ({})",
                    m.local_message_id, m.account_alias, m.subject, m.from.address
                );
            }
            for f in &found.partial_failures {
                eprintln!("! {}: {}", f.account_alias, f.reason);
            }
        }
        Command::Read { local_message_id } => {
            let d = agent.get_message(&local_message_id).await?;
            println!("From:    {}", d.summary.from.address);
            println!("Subject: {}", d.summary.subject);
            println!("Date:    {}", d.summary.received_at);
            println!("Account: {}", d.summary.account_alias);
            if !d.attachments.is_empty() {
                println!("Attachments:");
                for a in &d.attachments {
                    println!("  - {} ({} bytes, {})", a.filename, a.size_bytes, a.mime_type);
                }
            }
            println!("\n{}", d.body_text);
        }
        Command::Reconnect { account } => {
            let a = agent.reconnect_account(&account).await?;
            println!("Reconnected {} ({}).", a.alias, a.email);
        }
        Command::Permissions { account, draft } => {
            let mut acct = agent
                .list_accounts()?
                .into_iter()
                .find(|a| a.alias == account || a.id == account)
                .ok_or_else(|| anyhow::anyhow!("no account matching '{account}'"))?;
            if let Some(d) = draft {
                acct.permissions.modify = d;
            }
            let updated = agent.set_account_permissions(&acct.id, acct.permissions)?;
            println!(
                "{}: read={} draft={}",
                updated.alias, updated.permissions.read, updated.permissions.modify
            );
        }
        Command::DraftReply {
            local_message_id,
            body,
            reply_all,
        } => {
            let d = agent
                .create_draft_reply(&local_message_id, reply_all, &body)
                .await?;
            println!("Draft created: {} — \"{}\"", d.local_draft_id, d.subject);
        }
        Command::RemoveAccount { account } => {
            let email = agent.remove_account(&account)?;
            println!("Removed {account} ({email}).");
        }
        Command::InstallMcp { client } => {
            let exe = std::env::current_exe()?;
            let path = mailagent_core::install_mcp_client(&client, &exe)?;
            println!(
                "Registered Beeline with {client}: {}\nRestart {client} to load the tools.",
                path.display()
            );
        }
        Command::Mcp => mailagent_mcp::run_stdio(agent).await?,
        Command::Serve => {
            let socket = mailagent_core::default_socket_path()?;
            mailagent_control::run_uds(agent, &socket).await?;
        }
        Command::Ctl { method, params } => ctl(&method, params.as_deref()).await?,
    }

    Ok(())
}

/// Connect to the control daemon, send one JSON-RPC request, print the reply.
async fn ctl(method: &str, params: Option<&str>) -> anyhow::Result<()> {
    let socket = mailagent_core::default_socket_path()?;
    let mut stream = tokio::net::UnixStream::connect(&socket)
        .await
        .with_context(|| {
            format!(
                "connecting to {} — is `mailagent serve` running?",
                socket.display()
            )
        })?;

    let params: Value = match params {
        Some(p) => serde_json::from_str(p).context("--params must be valid JSON")?,
        None => json!({}),
    };
    let mut request = serde_json::to_string(&json!({
        "jsonrpc": "2.0", "id": 1, "method": method, "params": params
    }))?;
    request.push('\n');
    stream.write_all(request.as_bytes()).await?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;
    println!("{}", response.trim_end());
    Ok(())
}
