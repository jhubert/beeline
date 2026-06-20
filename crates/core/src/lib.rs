//! Core capability layer — the single facade the CLI, MCP server, and (later)
//! the control-API daemon all call. No business logic lives in those shells;
//! it lives here, so the safety/permission model has exactly one home.

pub mod auth;
pub mod config;
pub mod local_ids;
pub mod policy;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mailagent_providers::{gmail::GmailProvider, microsoft::MicrosoftProvider, MailProvider};
use mailagent_storage::{
    AuditEvent, Confirmation, Db, KeyringStore, MemorySecretStore, ProviderRef, SecretStore,
};
use mailagent_types::{
    AccountStatus, ConnectedAccount, MailSearchQuery, MessageDetail, MessageSummary, Permissions,
    Provider,
};
use serde::Serialize;

use crate::config::OAuthConfig;

const KEYRING_SERVICE: &str = "com.appcamp.beelinemailagent";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialFailure {
    pub account_alias: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub results: Vec<MessageSummary>,
    pub partial_failures: Vec<PartialFailure>,
}

pub struct MailAgent {
    db: Db,
    providers: HashMap<Provider, Arc<dyn MailProvider>>,
    /// OAuth refresh tokens live here, keyed by account id — never in the DB
    /// (SPEC.md §10.1, §19).
    secrets: Box<dyn SecretStore>,
}

impl MailAgent {
    /// Open against a SQLite file, persisting tokens in the OS keychain.
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        let db = Db::open(db_path)?;
        Ok(Self {
            db,
            providers: default_providers(),
            secrets: Box::new(KeyringStore::new(KEYRING_SERVICE)),
        })
    }

    /// In-memory store with no keychain access — for tests and headless CI.
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let db = Db::open_in_memory()?;
        Ok(Self {
            db,
            providers: default_providers(),
            secrets: Box::new(MemorySecretStore::default()),
        })
    }

    pub fn list_accounts(&self) -> anyhow::Result<Vec<ConnectedAccount>> {
        self.db.list_accounts()
    }

    // --- onboarding ----------------------------------------------------------

    /// Run the Google sign-in flow, persist the account, and store its refresh
    /// token in the Keychain. Defaults to read-only permission (SPEC.md §12.1).
    pub async fn add_gmail_account(
        &self,
        alias: Option<String>,
    ) -> anyhow::Result<ConnectedAccount> {
        let config = OAuthConfig::load()?;
        let identity = auth::google_authorize(&config).await?;

        let id = format!("acct_gmail_{}", slug(&identity.email));
        let alias = alias.unwrap_or_else(|| {
            identity
                .email
                .split('@')
                .next()
                .unwrap_or("gmail")
                .to_string()
        });

        // Refresh token to the Keychain (keyed by account id); never the DB.
        self.secrets.set(&id, &identity.refresh_token)?;

        let account = ConnectedAccount {
            id,
            provider: Provider::Gmail,
            alias,
            email: identity.email,
            display_name: None,
            enabled: true,
            permissions: Permissions {
                read: true,
                ..Default::default()
            },
            status: AccountStatus::Connected,
        };
        self.db.upsert_account(&account)?;
        self.db
            .record_audit(Some(&account.alias), Some(Provider::Gmail), "account_added", true, None)?;
        Ok(account)
    }

    /// Obtain a fresh access token for an account, refreshing from the stored
    /// refresh token. Stub providers (Microsoft, for now) get an empty token.
    async fn access_token(&self, account: &ConnectedAccount) -> anyhow::Result<String> {
        match account.provider {
            Provider::Gmail => {
                let config = OAuthConfig::load()?;
                let refresh = self
                    .secrets
                    .get(&account.id)?
                    .ok_or_else(|| anyhow::anyhow!("no stored credentials"))?;
                auth::google_access_token(&config, &refresh).await
            }
            _ => Ok(String::new()),
        }
    }

    /// Disconnect an account: delete its token from the Keychain and its row +
    /// local-id mappings from the DB. Accepts an alias or an account id.
    /// Returns the removed account's email.
    pub fn remove_account(&self, alias_or_id: &str) -> anyhow::Result<String> {
        let account = self
            .db
            .list_accounts()?
            .into_iter()
            .find(|a| a.alias == alias_or_id || a.id == alias_or_id)
            .ok_or_else(|| anyhow::anyhow!("no account matching '{alias_or_id}'"))?;

        let _ = self.secrets.delete(&account.id); // best-effort; demo rows have none
        self.db.delete_account(&account.id)?;
        self.db.record_audit(
            Some(&account.alias),
            Some(account.provider),
            "account_removed",
            true,
            None,
        )?;
        Ok(account.email)
    }

    // --- control-plane operations (driven by the CLI and GUI) ----------------

    /// Update an account's permission tiers (SPEC.md §12.1, §17.3) and log it.
    pub fn set_account_permissions(
        &self,
        account_id: &str,
        permissions: Permissions,
    ) -> anyhow::Result<ConnectedAccount> {
        let mut account = self
            .db
            .get_account(account_id)?
            .ok_or_else(|| anyhow::anyhow!("account not found: {account_id}"))?;
        account.permissions = permissions;
        self.db.upsert_account(&account)?;
        self.db.record_audit(
            Some(&account.alias),
            Some(account.provider),
            "permissions_changed",
            true,
            None,
        )?;
        Ok(account)
    }

    pub fn recent_audit(&self, limit: u32) -> anyhow::Result<Vec<AuditEvent>> {
        self.db.recent_audit(limit)
    }

    /// Record a pending mutation that needs explicit user approval. Returns the
    /// confirmation id the caller surfaces (e.g. MCP's `requiresConfirmation`).
    pub fn request_confirmation(
        &self,
        account_id: Option<&str>,
        action: &str,
        detail: Option<&str>,
    ) -> anyhow::Result<i64> {
        self.db.create_confirmation(account_id, action, detail)
    }

    pub fn pending_confirmations(&self) -> anyhow::Result<Vec<Confirmation>> {
        self.db.list_confirmations("pending")
    }

    pub fn resolve_confirmation(&self, id: i64, approve: bool) -> anyhow::Result<bool> {
        self.db.resolve_confirmation(id, approve)
    }

    /// Cross-account search (SPEC.md §14.1). `selector` is an alias or "all".
    /// Per-account failures become partial failures rather than failing the
    /// whole search. Provider calls will run concurrently once real adapters
    /// land (Phase 1); the stubs are cheap enough to run sequentially here.
    pub async fn search(
        &self,
        selector: &str,
        query: &MailSearchQuery,
    ) -> anyhow::Result<SearchResults> {
        let mut results = Vec::new();
        let mut partial_failures = Vec::new();

        for account in self.db.list_accounts()? {
            if !account.enabled {
                continue;
            }
            if selector != "all" && selector != account.alias {
                continue;
            }
            if !policy::can_read(&account) {
                partial_failures.push(PartialFailure {
                    account_alias: account.alias.clone(),
                    reason: AccountStatus::PermissionMissing.as_str().to_string(),
                });
                continue;
            }
            let Some(provider) = self.providers.get(&account.provider) else {
                continue;
            };

            // A token failure (revoked/expired refresh token) is reported as a
            // per-account partial failure, not a whole-search failure.
            let token = match self.access_token(&account).await {
                Ok(token) => token,
                Err(e) => {
                    partial_failures.push(PartialFailure {
                        account_alias: account.alias.clone(),
                        reason: format!("{}: {e}", AccountStatus::NeedsReconnect.as_str()),
                    });
                    continue;
                }
            };

            match provider.search_messages(&account, &token, query).await {
                Ok(hits) => {
                    for (provider_message_id, mut summary) in hits {
                        let local_id = self.db.mint_local_id(&ProviderRef {
                            provider: account.provider,
                            account_id: account.id.clone(),
                            provider_message_id,
                        })?;
                        summary.local_message_id = local_id;
                        summary.account_id = account.id.clone();
                        summary.account_alias = account.alias.clone();
                        summary.account_email = account.email.clone();
                        results.push(summary);
                    }
                }
                Err(e) => partial_failures.push(PartialFailure {
                    account_alias: account.alias.clone(),
                    reason: e.to_string(),
                }),
            }
        }

        // Merge sort by received date, newest first (SPEC.md §14.1).
        results.sort_by(|a, b| b.received_at.cmp(&a.received_at));

        Ok(SearchResults {
            results,
            partial_failures,
        })
    }

    pub async fn get_message(&self, local_message_id: &str) -> anyhow::Result<MessageDetail> {
        let reference = self
            .db
            .resolve_local_id(local_message_id)?
            .ok_or_else(|| anyhow::anyhow!("unknown localMessageId: {local_message_id}"))?;

        let account = self
            .db
            .get_account(&reference.account_id)?
            .ok_or_else(|| anyhow::anyhow!("account not found for message"))?;

        if !policy::can_read(&account) {
            anyhow::bail!("read permission not enabled for account {}", account.alias);
        }

        let provider = self
            .providers
            .get(&reference.provider)
            .ok_or_else(|| anyhow::anyhow!("provider not available"))?;

        let token = self.access_token(&account).await?;
        let mut detail = provider
            .get_message(&account, &token, &reference.provider_message_id)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

        detail.summary.local_message_id = local_message_id.to_string();
        detail.summary.account_id = account.id.clone();
        detail.summary.account_alias = account.alias.clone();
        detail.summary.account_email = account.email.clone();
        Ok(detail)
    }
}

/// Lowercase alphanumeric slug for building stable account ids from emails.
fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

fn default_providers() -> HashMap<Provider, Arc<dyn MailProvider>> {
    let mut providers: HashMap<Provider, Arc<dyn MailProvider>> = HashMap::new();
    providers.insert(Provider::Gmail, Arc::new(GmailProvider::new()));
    providers.insert(Provider::Microsoft, Arc::new(MicrosoftProvider::new()));
    providers
}

/// Default on-disk location for the SQLite store: `~/.mailagent/mailagent.sqlite`.
/// Phase 2 will move this to the macOS Application Support directory.
pub fn default_db_path() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("mailagent.sqlite"))
}

/// Unix-domain socket for the control API (SPEC.md §6.2). Lives under the
/// user-owned data dir; bound 0600 so only this user can connect — never a
/// network listener.
pub fn default_socket_path() -> anyhow::Result<PathBuf> {
    Ok(data_dir()?.join("control.sock"))
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    let dir = Path::new(&home).join(".mailagent");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
