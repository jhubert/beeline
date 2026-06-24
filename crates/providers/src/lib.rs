//! Provider abstraction (SPEC.md §7). The rest of the app does not care whether
//! a message comes from Microsoft or Gmail — it only sees this trait.
//!
//! Phase 0 ships stubbed adapters that return canned data so the MCP path runs
//! end-to-end before any OAuth client registration exists. Replace the stub
//! bodies with the reqwest-based wrappers (see the Gmail `search_messages`
//! sketch) once client IDs are available.

use async_trait::async_trait;
use mailagent_types::{
    ConnectedAccount, DraftInput, MailSearchQuery, MessageDetail, MessageSummary, Provider,
};

/// A created draft's provider id + the subject it ended up with.
pub struct RawDraft {
    pub provider_draft_id: String,
    pub subject: String,
}

pub mod gmail;
pub mod microsoft;

/// Provider failures map directly onto `AccountStatus` (SPEC.md §18) so the
/// cross-account search layer can report them as per-account partial failures.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("needs_reconnect")]
    NeedsReconnect,
    #[error("permission_missing")]
    PermissionMissing,
    #[error("admin_approval_required")]
    AdminApprovalRequired,
    #[error("provider_rate_limited")]
    RateLimited,
    #[error("provider_unavailable")]
    Unavailable,
    #[error("unknown_error: {0}")]
    Unknown(String),
}

/// A search hit before local-id assignment: the provider's own message id
/// paired with the normalized summary. The core mints a `local_msg_*` id from
/// the provider id and stamps it onto the summary (SPEC.md §9) — raw provider
/// ids never leave this crate boundary toward the agent.
pub type RawHit = (String, MessageSummary);

#[async_trait]
pub trait MailProvider: Send + Sync {
    fn provider_name(&self) -> Provider;

    /// `access_token` is a fresh bearer token the core has already obtained
    /// (refreshing from the Keychain as needed). Providers are stateless about
    /// auth — they just use the token they're handed.
    async fn search_messages(
        &self,
        account: &ConnectedAccount,
        access_token: &str,
        query: &MailSearchQuery,
    ) -> Result<Vec<RawHit>, ProviderError>;

    async fn get_message(
        &self,
        account: &ConnectedAccount,
        access_token: &str,
        provider_message_id: &str,
    ) -> Result<MessageDetail, ProviderError>;

    /// Create a new draft (SPEC.md §13.5). Default impl: unsupported.
    async fn create_draft(
        &self,
        _account: &ConnectedAccount,
        _access_token: &str,
        _input: &DraftInput,
    ) -> Result<RawDraft, ProviderError> {
        Err(ProviderError::Unknown(
            "draft creation not supported for this provider yet".into(),
        ))
    }

    /// Create a draft reply to an existing message (SPEC.md §13.6). Default
    /// impl: unsupported.
    async fn create_draft_reply(
        &self,
        _account: &ConnectedAccount,
        _access_token: &str,
        _provider_message_id: &str,
        _reply_all: bool,
        _body_text: &str,
    ) -> Result<RawDraft, ProviderError> {
        Err(ProviderError::Unknown(
            "draft replies not supported for this provider yet".into(),
        ))
    }
}
