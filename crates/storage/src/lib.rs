//! Local persistence (SPEC.md §10). SQLite for account records, local-id
//! mappings, and audit logs; the OS secret store for OAuth tokens. Nothing here
//! leaves the machine.

pub mod db;
pub mod secrets;

pub use db::Db;
pub use secrets::{KeyringStore, MemorySecretStore, SecretStore};

use mailagent_types::Provider;
use serde::Serialize;

/// A resolved local id → its provider-scoped origin (SPEC.md §9). Lives here
/// (not in `core`) because the DB both mints and resolves these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRef {
    pub provider: Provider,
    pub account_id: String,
    pub provider_message_id: String,
}

/// A logged mutation event (SPEC.md §17.5). Never carries message bodies.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub id: i64,
    pub at: String,
    pub account_alias: Option<String>,
    pub provider: Option<String>,
    pub action: String,
    pub success: bool,
    pub error_category: Option<String>,
}

/// A pending mutation awaiting user approval (SPEC.md §12.2, Open Question #6).
/// Confirmation is core state: the MCP process records one and returns
/// `requiresConfirmation`; the CLI or GUI resolves it over the control API.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirmation {
    pub id: i64,
    pub created_at: String,
    pub account_id: Option<String>,
    pub action: String,
    /// JSON summary of the pending action — metadata only, never bodies.
    pub detail: Option<String>,
    pub status: String,
}
