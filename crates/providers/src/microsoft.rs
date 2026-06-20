//! Microsoft Graph adapter. Search via `$search` (KQL) when there are text
//! criteria, else `$filter` + `$orderby`. HTTP errors map onto `ProviderError`.

use async_trait::async_trait;
use mailagent_types::{
    AttachmentSummary, ConnectedAccount, EmailAddress, MailSearchQuery, MessageDetail,
    MessageSummary, Provider,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::{MailProvider, ProviderError, RawHit};

const GRAPH: &str = "https://graph.microsoft.com/v1.0/me";
const SELECT: &str =
    "id,subject,from,toRecipients,ccRecipients,receivedDateTime,sentDateTime,bodyPreview,isRead,hasAttachments";

#[derive(Default)]
pub struct MicrosoftProvider {
    http: reqwest::Client,
}

impl MicrosoftProvider {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
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
        access_token: &str,
        query: &MailSearchQuery,
    ) -> Result<Vec<RawHit>, ProviderError> {
        let limit = query.limit.unwrap_or(20).clamp(1, 100);
        let url = build_url(query, limit);
        let list: MessageList = get_json(&self.http, &url, access_token).await?;
        Ok(list
            .value
            .into_iter()
            .map(|m| (m.id.clone(), to_summary(&m)))
            .collect())
    }

    async fn get_message(
        &self,
        _account: &ConnectedAccount,
        access_token: &str,
        provider_message_id: &str,
    ) -> Result<MessageDetail, ProviderError> {
        let url = format!("{GRAPH}/messages/{provider_message_id}?$select={SELECT},body");
        let message: GraphMessage = get_json(&self.http, &url, access_token).await?;

        let mut attachments = Vec::new();
        if message.has_attachments {
            let url = format!(
                "{GRAPH}/messages/{provider_message_id}/attachments?$select=id,name,size,contentType"
            );
            if let Ok(list) = get_json::<AttachmentList>(&self.http, &url, access_token).await {
                attachments = list
                    .value
                    .into_iter()
                    .map(|a| AttachmentSummary {
                        local_attachment_id: String::new(), // minted by core when listed
                        filename: a.name.unwrap_or_default(),
                        size_bytes: a.size.unwrap_or(0),
                        mime_type: a.content_type.unwrap_or_default(),
                    })
                    .collect();
            }
        }

        let (body_text, body_html_available) = match &message.body {
            Some(b) if b.content_type.eq_ignore_ascii_case("text") => (b.content.clone(), false),
            Some(b) => (strip_html(&b.content), true),
            None => (message.body_preview.clone(), false),
        };

        Ok(MessageDetail {
            summary: to_summary(&message),
            body_text,
            body_html_available,
            body_html: None,
            attachments,
        })
    }
}

// --- Graph wire types -------------------------------------------------------

#[derive(Deserialize)]
struct MessageList {
    #[serde(default)]
    value: Vec<GraphMessage>,
}

#[derive(Deserialize)]
struct GraphMessage {
    id: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    from: Option<Recipient>,
    #[serde(default, rename = "toRecipients")]
    to_recipients: Vec<Recipient>,
    #[serde(default, rename = "ccRecipients")]
    cc_recipients: Vec<Recipient>,
    #[serde(default, rename = "receivedDateTime")]
    received: String,
    #[serde(default, rename = "sentDateTime")]
    sent: Option<String>,
    #[serde(default, rename = "bodyPreview")]
    body_preview: String,
    #[serde(default, rename = "isRead")]
    is_read: bool,
    #[serde(default, rename = "hasAttachments")]
    has_attachments: bool,
    #[serde(default)]
    body: Option<Body>,
}

#[derive(Deserialize)]
struct Recipient {
    #[serde(rename = "emailAddress")]
    email_address: Option<Address>,
}

#[derive(Deserialize)]
struct Address {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    address: Option<String>,
}

#[derive(Deserialize)]
struct Body {
    #[serde(default, rename = "contentType")]
    content_type: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct AttachmentList {
    #[serde(default)]
    value: Vec<GraphAttachment>,
}

#[derive(Deserialize)]
struct GraphAttachment {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default, rename = "contentType")]
    content_type: Option<String>,
}

// --- normalization ----------------------------------------------------------

fn to_summary(m: &GraphMessage) -> MessageSummary {
    MessageSummary {
        provider: Provider::Microsoft,
        from: m
            .from
            .as_ref()
            .and_then(|r| r.email_address.as_ref())
            .map(to_address)
            .unwrap_or_default(),
        to: opt_addresses(&m.to_recipients),
        cc: opt_addresses(&m.cc_recipients),
        subject: m.subject.clone(),
        received_at: m.received.clone(),
        sent_at: m.sent.clone(),
        preview: m.body_preview.clone(),
        unread: !m.is_read,
        has_attachments: m.has_attachments,
        ..Default::default()
    }
}

fn to_address(a: &Address) -> EmailAddress {
    EmailAddress {
        name: a.name.clone().filter(|s| !s.is_empty()),
        address: a.address.clone().unwrap_or_default(),
    }
}

fn opt_addresses(recipients: &[Recipient]) -> Option<Vec<EmailAddress>> {
    let addrs: Vec<_> = recipients
        .iter()
        .filter_map(|r| r.email_address.as_ref())
        .map(to_address)
        .collect();
    (!addrs.is_empty()).then_some(addrs)
}

// --- query translation (SPEC §14.2) -----------------------------------------

fn build_url(query: &MailSearchQuery, limit: u32) -> String {
    let mut url = format!("{GRAPH}/messages?$top={limit}&$select={SELECT}");
    let kql = build_kql(query);
    if !kql.is_empty() {
        // $search can't be combined with $orderby/$filter; its value is quoted.
        url.push_str(&format!("&$search=%22{}%22", urlencode(&kql)));
    } else {
        url.push_str("&$orderby=receivedDateTime%20desc");
        let filter = build_filter(query);
        if !filter.is_empty() {
            url.push_str(&format!("&$filter={}", urlencode(&filter)));
        }
    }
    url
}

fn build_kql(q: &MailSearchQuery) -> String {
    let mut parts = Vec::new();
    if let Some(text) = &q.free_text {
        if !text.is_empty() {
            parts.push(text.clone());
        }
    }
    if let Some(from) = &q.from {
        parts.push(format!("from:{from}"));
    }
    if let Some(to) = &q.to {
        parts.push(format!("to:{to}"));
    }
    if let Some(subject) = &q.subject {
        parts.push(format!("subject:{subject}"));
    }
    parts.join(" ")
}

fn build_filter(q: &MailSearchQuery) -> String {
    let mut parts = Vec::new();
    if q.unread_only == Some(true) {
        parts.push("isRead eq false".to_string());
    }
    if q.has_attachments == Some(true) {
        parts.push("hasAttachments eq true".to_string());
    }
    if let Some(since) = &q.since {
        parts.push(format!("receivedDateTime ge {since}T00:00:00Z"));
    }
    if let Some(before) = &q.before {
        parts.push(format!("receivedDateTime lt {before}T00:00:00Z"));
    }
    parts.join(" and ")
}

// --- HTTP helpers -----------------------------------------------------------

async fn get_json<T: DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<T, ProviderError> {
    let response = http
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| ProviderError::Unavailable)?;

    let status = response.status();
    if status.is_success() {
        return response
            .json::<T>()
            .await
            .map_err(|e| ProviderError::Unknown(e.to_string()));
    }
    Err(match status.as_u16() {
        401 => ProviderError::NeedsReconnect,
        403 => ProviderError::PermissionMissing,
        429 => ProviderError::RateLimited,
        500..=599 => ProviderError::Unavailable,
        other => ProviderError::Unknown(format!("graph returned HTTP {other}")),
    })
}

/// Crude HTML→text for message bodies (Graph returns HTML by default).
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
