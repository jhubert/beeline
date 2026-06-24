//! Gmail adapter (Gmail API). Two-step search (`messages.list` → per-id
//! `messages.get`); HTTP errors map onto `ProviderError` so the core can report
//! them as per-account partial failures or `needs_reconnect`.

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;
use mailagent_types::{
    AttachmentSummary, ConnectedAccount, DraftInput, EmailAddress, MailSearchQuery, MessageDetail,
    MessageSummary, Provider,
};
use serde::{Deserialize, Serialize};

use crate::{MailProvider, ProviderError, RawDraft, RawHit};

const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

#[derive(Default)]
pub struct GmailProvider {
    http: reqwest::Client,
}

impl GmailProvider {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl MailProvider for GmailProvider {
    fn provider_name(&self) -> Provider {
        Provider::Gmail
    }

    async fn search_messages(
        &self,
        _account: &ConnectedAccount,
        access_token: &str,
        query: &MailSearchQuery,
    ) -> Result<Vec<RawHit>, ProviderError> {
        let limit = query.limit.unwrap_or(20).clamp(1, 100);
        let q = build_query(query);
        let url = format!(
            "{BASE}/messages?maxResults={limit}&q={}",
            urlencode(&q)
        );
        let list: ListResponse = get_json(&self.http, &url, access_token).await?;

        let mut hits = Vec::new();
        for reference in list.messages.unwrap_or_default() {
            let url = format!(
                "{BASE}/messages/{}?format=metadata\
                 &metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date",
                reference.id
            );
            let message: GmailMessage = get_json(&self.http, &url, access_token).await?;
            hits.push((reference.id.clone(), to_summary(&message)));
        }
        Ok(hits)
    }

    async fn get_message(
        &self,
        _account: &ConnectedAccount,
        access_token: &str,
        provider_message_id: &str,
    ) -> Result<MessageDetail, ProviderError> {
        let url = format!("{BASE}/messages/{provider_message_id}?format=full");
        let message: GmailMessage = get_json(&self.http, &url, access_token).await?;
        Ok(to_detail(&message))
    }

    async fn create_draft(
        &self,
        _account: &ConnectedAccount,
        access_token: &str,
        input: &DraftInput,
    ) -> Result<RawDraft, ProviderError> {
        let mime = build_mime(&input.to, &input.cc, &input.bcc, &input.subject, &input.body_text, &[]);
        let request = DraftCreate {
            message: DraftMessage {
                raw: URL_SAFE.encode(mime.as_bytes()),
                thread_id: None,
            },
        };
        let created: DraftResponse =
            post_json(&self.http, &format!("{BASE}/drafts"), access_token, &request).await?;
        Ok(RawDraft {
            provider_draft_id: created.id,
            subject: input.subject.clone(),
        })
    }

    async fn create_draft_reply(
        &self,
        account: &ConnectedAccount,
        access_token: &str,
        provider_message_id: &str,
        reply_all: bool,
        body_text: &str,
    ) -> Result<RawDraft, ProviderError> {
        // Pull the headers we need to thread the reply correctly.
        let url = format!(
            "{BASE}/messages/{provider_message_id}?format=metadata\
             &metadataHeaders=From&metadataHeaders=To&metadataHeaders=Cc\
             &metadataHeaders=Subject&metadataHeaders=Message-ID"
        );
        let original: GmailMessage = get_json(&self.http, &url, access_token).await?;
        let payload = original.payload.as_ref();
        let header_value = |name: &str| {
            payload
                .and_then(|p| header(p, name))
                .unwrap_or("")
                .to_string()
        };

        let from = header_value("From");
        let subject = header_value("Subject");
        let message_id = header_value("Message-ID");

        let to = vec![from.clone()];
        let mut cc = Vec::new();
        if reply_all {
            let self_addr = account.email.to_lowercase();
            for list in [header_value("To"), header_value("Cc")] {
                for addr in list.split(',').map(str::trim).filter(|a| !a.is_empty()) {
                    let lower = addr.to_lowercase();
                    if !lower.contains(&self_addr) && !addr.eq_ignore_ascii_case(&from) {
                        cc.push(addr.to_string());
                    }
                }
            }
        }

        let reply_subject = if subject.to_lowercase().starts_with("re:") {
            subject
        } else {
            format!("Re: {subject}")
        };
        let mut extra: Vec<(&str, String)> = Vec::new();
        if !message_id.is_empty() {
            extra.push(("In-Reply-To", message_id.clone()));
            extra.push(("References", message_id));
        }

        let mime = build_mime(&to, &cc, &[], &reply_subject, body_text, &extra);
        let request = DraftCreate {
            message: DraftMessage {
                raw: URL_SAFE.encode(mime.as_bytes()),
                thread_id: (!original.thread_id.is_empty()).then(|| original.thread_id.clone()),
            },
        };
        let created: DraftResponse =
            post_json(&self.http, &format!("{BASE}/drafts"), access_token, &request).await?;
        Ok(RawDraft {
            provider_draft_id: created.id,
            subject: reply_subject,
        })
    }
}

#[derive(Serialize)]
struct DraftCreate {
    message: DraftMessage,
}

#[derive(Serialize)]
struct DraftMessage {
    raw: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "threadId")]
    thread_id: Option<String>,
}

#[derive(Deserialize)]
struct DraftResponse {
    id: String,
}

/// Minimal RFC 822 text/plain message for the Gmail `raw` field.
fn build_mime(
    to: &[String],
    cc: &[String],
    bcc: &[String],
    subject: &str,
    body: &str,
    extra_headers: &[(&str, String)],
) -> String {
    let mut m = String::new();
    if !to.is_empty() {
        m.push_str(&format!("To: {}\r\n", to.join(", ")));
    }
    if !cc.is_empty() {
        m.push_str(&format!("Cc: {}\r\n", cc.join(", ")));
    }
    if !bcc.is_empty() {
        m.push_str(&format!("Bcc: {}\r\n", bcc.join(", ")));
    }
    m.push_str(&format!("Subject: {subject}\r\n"));
    for (k, v) in extra_headers {
        m.push_str(&format!("{k}: {v}\r\n"));
    }
    m.push_str("MIME-Version: 1.0\r\n");
    m.push_str("Content-Type: text/plain; charset=\"UTF-8\"\r\n\r\n");
    m.push_str(body);
    m
}

// --- Gmail API wire types ---------------------------------------------------

#[derive(Deserialize)]
struct ListResponse {
    messages: Option<Vec<MessageRef>>,
}

#[derive(Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Deserialize)]
struct GmailMessage {
    #[serde(default, rename = "threadId")]
    thread_id: String,
    #[serde(default, rename = "labelIds")]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    payload: Option<Part>,
}

#[derive(Deserialize, Default)]
struct Part {
    #[serde(default, rename = "mimeType")]
    mime_type: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    headers: Vec<Header>,
    #[serde(default)]
    body: Body,
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Deserialize, Default)]
struct Body {
    #[serde(default)]
    size: u64,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

// --- normalization ----------------------------------------------------------

fn header<'a>(part: &'a Part, name: &str) -> Option<&'a str> {
    part.headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn to_summary(message: &GmailMessage) -> MessageSummary {
    let payload = message.payload.as_ref();
    let get = |name: &str| payload.and_then(|p| header(p, name)).unwrap_or("");
    MessageSummary {
        provider: Provider::Gmail,
        from: EmailAddress::parse(get("From")),
        subject: get("Subject").to_string(),
        received_at: get("Date").to_string(),
        preview: message.snippet.clone(),
        unread: message.label_ids.iter().any(|l| l == "UNREAD"),
        has_attachments: payload.map(has_attachment).unwrap_or(false),
        labels: (!message.label_ids.is_empty()).then(|| message.label_ids.clone()),
        ..Default::default()
    }
}

fn to_detail(message: &GmailMessage) -> MessageDetail {
    let mut attachments = Vec::new();
    let mut body_text = String::new();
    if let Some(payload) = &message.payload {
        collect_parts(payload, &mut body_text, &mut attachments);
    }
    MessageDetail {
        summary: to_summary(message),
        body_text,
        body_html_available: message
            .payload
            .as_ref()
            .map(|p| contains_mime(p, "text/html"))
            .unwrap_or(false),
        body_html: None,
        attachments,
    }
}

fn has_attachment(part: &Part) -> bool {
    !part.filename.is_empty() || part.parts.iter().any(has_attachment)
}

fn contains_mime(part: &Part, mime: &str) -> bool {
    part.mime_type == mime || part.parts.iter().any(|p| contains_mime(p, mime))
}

/// Walk the MIME tree: capture the first text/plain body, list named parts as
/// attachment metadata (we never download attachment contents here — SPEC §15).
fn collect_parts(part: &Part, body_text: &mut String, attachments: &mut Vec<AttachmentSummary>) {
    if !part.filename.is_empty() {
        attachments.push(AttachmentSummary {
            local_attachment_id: String::new(), // minted by core when listed
            filename: part.filename.clone(),
            size_bytes: part.body.size,
            mime_type: part.mime_type.clone(),
        });
    }
    if part.mime_type == "text/plain" && body_text.is_empty() {
        if let Some(data) = &part.body.data {
            if let Ok(bytes) = URL_SAFE.decode(data) {
                *body_text = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
    }
    for child in &part.parts {
        collect_parts(child, body_text, attachments);
    }
}

/// Translate the normalized query into Gmail search syntax (SPEC §14.2).
fn build_query(query: &MailSearchQuery) -> String {
    let mut parts = Vec::new();
    if let Some(text) = &query.free_text {
        if !text.is_empty() {
            parts.push(text.clone());
        }
    }
    if let Some(from) = &query.from {
        parts.push(format!("from:{from}"));
    }
    if let Some(to) = &query.to {
        parts.push(format!("to:{to}"));
    }
    if let Some(subject) = &query.subject {
        parts.push(format!("subject:{subject}"));
    }
    if let Some(since) = &query.since {
        parts.push(format!("after:{}", since.replace('-', "/")));
    }
    if let Some(before) = &query.before {
        parts.push(format!("before:{}", before.replace('-', "/")));
    }
    if query.unread_only == Some(true) {
        parts.push("is:unread".to_string());
    }
    if query.has_attachments == Some(true) {
        parts.push("has:attachment".to_string());
    }
    parts.join(" ")
}

// --- HTTP helper ------------------------------------------------------------

async fn get_json<T: serde::de::DeserializeOwned>(
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
    Err(map_status(status))
}

async fn post_json<T: serde::de::DeserializeOwned, B: Serialize>(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
    body: &B,
) -> Result<T, ProviderError> {
    let response = http
        .post(url)
        .bearer_auth(access_token)
        .json(body)
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
    Err(map_status(status))
}

fn map_status(status: reqwest::StatusCode) -> ProviderError {
    match status.as_u16() {
        401 | 403 => ProviderError::NeedsReconnect,
        429 => ProviderError::RateLimited,
        500..=599 => ProviderError::Unavailable,
        other => ProviderError::Unknown(format!("gmail returned HTTP {other}")),
    }
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
