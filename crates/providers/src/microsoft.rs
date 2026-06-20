//! Microsoft Graph adapter. STUB: returns canned data. Replace with the reqwest
//! wrapper (Graph `/me/messages?$search=...`, `/me/messages/{id}`, delegated
//! OAuth token refresh, errors mapped to `ProviderError`).

use async_trait::async_trait;
use mailagent_types::{
    ConnectedAccount, EmailAddress, MailSearchQuery, MessageDetail, MessageSummary, Provider,
};

use crate::{MailProvider, ProviderError, RawHit};

#[derive(Default)]
pub struct MicrosoftProvider;

impl MicrosoftProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MailProvider for MicrosoftProvider {
    fn provider_name(&self) -> Provider {
        Provider::Microsoft
    }

    async fn search_messages(
        &self,
        _account: &ConnectedAccount,
        _access_token: &str,
        _query: &MailSearchQuery,
    ) -> Result<Vec<RawHit>, ProviderError> {
        // TODO(phase0->1): real Microsoft Graph call.
        Ok(vec![(
            "AAMkAGI2-msg-1".to_string(),
            MessageSummary {
                provider: Provider::Microsoft,
                from: EmailAddress::parse("Dana <dana@company.com>"),
                subject: "Q3 planning".to_string(),
                received_at: "2026-06-10T09:05:00Z".to_string(),
                preview: "Here is the draft plan for next quarter...".to_string(),
                unread: false,
                has_attachments: false,
                folder_name: Some("Inbox".to_string()),
                ..Default::default()
            },
        )])
    }

    async fn get_message(
        &self,
        _account: &ConnectedAccount,
        _access_token: &str,
        provider_message_id: &str,
    ) -> Result<MessageDetail, ProviderError> {
        Ok(MessageDetail {
            summary: MessageSummary {
                provider: Provider::Microsoft,
                from: EmailAddress::parse("Dana <dana@company.com>"),
                subject: "Q3 planning".to_string(),
                received_at: "2026-06-10T09:05:00Z".to_string(),
                preview: "Here is the draft plan for next quarter...".to_string(),
                unread: false,
                has_attachments: false,
                folder_name: Some("Inbox".to_string()),
                ..Default::default()
            },
            body_text: format!("(stub Graph body for {provider_message_id})"),
            body_html_available: false,
            body_html: None,
            attachments: vec![],
        })
    }
}
