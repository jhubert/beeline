//! Normalized data model shared across the CLI, MCP server, core, and (via
//! generated bindings) the optional GUI client. JSON is camelCase to match the
//! MCP tool I/O in SPEC.md §13. See SPEC.md §8.
//!
//! These types deliberately carry NO raw provider IDs — the agent only ever
//! sees `local_*` ids minted by the core's local-id layer (SPEC.md §9).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Gmail,
    Microsoft,
    Imap,
}

impl Provider {
    /// Stable token used for persistence (matches the lowercase JSON form).
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Gmail => "gmail",
            Provider::Microsoft => "microsoft",
            Provider::Imap => "imap",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "gmail" => Some(Provider::Gmail),
            "microsoft" => Some(Provider::Microsoft),
            "imap" => Some(Provider::Imap),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub address: String,
}

impl EmailAddress {
    /// Naive `"Name <addr@host>"` parser. Good enough for summaries; the
    /// provider adapters will hand us structured addresses where available.
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        if let (Some(lt), Some(gt)) = (raw.find('<'), raw.find('>')) {
            if gt > lt {
                let name = raw[..lt].trim().trim_matches('"').trim().to_string();
                let address = raw[lt + 1..gt].trim().to_string();
                return Self {
                    name: (!name.is_empty()).then_some(name),
                    address,
                };
            }
        }
        Self {
            name: None,
            address: raw.trim().to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub read: bool,
    pub modify: bool,
    pub send: bool,
    pub attachments: bool,
}

/// Account lifecycle states (SPEC.md §18). These double as the failure
/// `reason` surfaced in cross-account partial failures (SPEC.md §14.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Connected,
    NeedsReconnect,
    PermissionMissing,
    AdminApprovalRequired,
    ProviderRateLimited,
    ProviderUnavailable,
    UnknownError,
}

impl AccountStatus {
    /// snake_case token, matching the JSON form and used for persistence and as
    /// the `reason` in cross-account partial failures (SPEC.md §14.1).
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountStatus::Connected => "connected",
            AccountStatus::NeedsReconnect => "needs_reconnect",
            AccountStatus::PermissionMissing => "permission_missing",
            AccountStatus::AdminApprovalRequired => "admin_approval_required",
            AccountStatus::ProviderRateLimited => "provider_rate_limited",
            AccountStatus::ProviderUnavailable => "provider_unavailable",
            AccountStatus::UnknownError => "unknown_error",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "connected" => Some(AccountStatus::Connected),
            "needs_reconnect" => Some(AccountStatus::NeedsReconnect),
            "permission_missing" => Some(AccountStatus::PermissionMissing),
            "admin_approval_required" => Some(AccountStatus::AdminApprovalRequired),
            "provider_rate_limited" => Some(AccountStatus::ProviderRateLimited),
            "provider_unavailable" => Some(AccountStatus::ProviderUnavailable),
            "unknown_error" => Some(AccountStatus::UnknownError),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedAccount {
    pub id: String,
    pub provider: Provider,
    pub alias: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub enabled: bool,
    pub permissions: Permissions,
    pub status: AccountStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub local_message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_thread_id: Option<String>,
    pub provider: Provider,
    pub account_id: String,
    pub account_alias: String,
    pub account_email: String,
    pub from: EmailAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Vec<EmailAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cc: Option<Vec<EmailAddress>>,
    pub subject: String,
    pub received_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<String>,
    pub preview: String,
    pub unread: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub important: Option<bool>,
    pub has_attachments: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentSummary {
    pub local_attachment_id: String,
    pub filename: String,
    pub size_bytes: u64,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub body_text: String,
    pub body_html_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    pub attachments: Vec<AttachmentSummary>,
}

/// Internal normalized query (SPEC.md §14.2). The MCP layer accepts simple
/// natural parameters; adapters translate to provider-specific search syntax.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailSearchQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_attachments: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}
