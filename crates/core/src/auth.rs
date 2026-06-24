//! Google OAuth: authorization-code + PKCE with a loopback redirect (SPEC.md
//! §11.1–11.3). Public-client flow — the desktop "client secret" is sent in the
//! token exchange (Google requires it for desktop clients) but PKCE is what
//! actually secures the exchange. Refresh tokens are returned to the caller,
//! which stores them in the Keychain; access tokens are fetched on demand.

use anyhow::{anyhow, Context};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::config::OAuthConfig;

const GOOGLE_AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN: &str = "https://oauth2.googleapis.com/token";
// Read-only access + draft creation. `gmail.compose` is the minimal scope that
// allows creating drafts (there is no draft-only scope). We never expose a send
// tool — drafts are reviewed and sent by the user in their mail app.
const GMAIL_SCOPE: &str =
    "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose";
const GMAIL_PROFILE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";

// Multi-tenant + personal accounts (the "common" authority) so one app covers
// both Outlook.com (consumer MSA) and Microsoft 365 (work/school). SPEC §11.1.
const MS_AUTH: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const MS_TOKEN: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
// Mail.ReadWrite covers both reading and draft creation (Graph creates drafts
// via POST /me/messages and /createReply). No send permission is requested.
const MS_SCOPE: &str = "openid profile offline_access Mail.ReadWrite";

pub struct ConnectedIdentity {
    pub email: String,
    pub refresh_token: String,
}

/// A failed access-token refresh. `needs_reconnect` is true when the provider
/// rejected the refresh token itself (expired/revoked) — distinct from a
/// transient network or config error, which shouldn't flip an account's status.
#[derive(Debug)]
pub struct TokenRefreshError {
    pub needs_reconnect: bool,
    pub detail: String,
}

impl std::fmt::Display for TokenRefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for TokenRefreshError {}

/// A freshly refreshed access token plus how long (seconds) it is valid.
pub struct AccessToken {
    pub token: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

/// Interpret a token-endpoint response for a refresh request. A 400/401 means
/// the refresh token was rejected → the account needs reconnecting.
async fn refresh_response(response: reqwest::Response) -> Result<AccessToken, TokenRefreshError> {
    let status = response.status();
    if status.is_success() {
        let token: TokenResponse = response.json().await.map_err(|e| TokenRefreshError {
            needs_reconnect: false,
            detail: e.to_string(),
        })?;
        return Ok(AccessToken {
            token: token.access_token,
            expires_in: token.expires_in,
        });
    }
    let body = response.text().await.unwrap_or_default();
    Err(TokenRefreshError {
        needs_reconnect: matches!(status.as_u16(), 400 | 401),
        detail: format!("token refresh failed ({status}): {body}"),
    })
}

fn send_error(e: reqwest::Error) -> TokenRefreshError {
    TokenRefreshError {
        needs_reconnect: false,
        detail: e.to_string(),
    }
}

/// Run the Google loopback + PKCE authorization flow: open the browser, wait for
/// the redirect, exchange the code, and return the user's email + refresh token.
pub async fn google_authorize(config: &OAuthConfig) -> anyhow::Result<ConnectedIdentity> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());

    let (verifier, challenge) = pkce();
    let state = random_b64url(16);

    let auth_url = format!(
        "{GOOGLE_AUTH}?response_type=code&client_id={cid}&redirect_uri={redir}\
         &scope={scope}&code_challenge={chal}&code_challenge_method=S256\
         &access_type=offline&prompt=consent&state={state}",
        cid = urlencode(&config.google_client_id),
        redir = urlencode(&redirect),
        scope = urlencode(GMAIL_SCOPE),
        chal = challenge,
        state = state,
    );

    eprintln!("Opening your browser to sign in to Google...");
    eprintln!("If it doesn't open, paste this URL:\n{auth_url}\n");
    let _ = std::process::Command::new("open").arg(&auth_url).spawn();

    let (code, returned_state) = wait_for_redirect(&listener).await?;
    if returned_state != state {
        return Err(anyhow!("OAuth state mismatch — aborting"));
    }

    let client = reqwest::Client::new();
    let token: TokenResponse = client
        .post(GOOGLE_TOKEN)
        .form(&[
            ("code", code.as_str()),
            ("client_id", &config.google_client_id),
            ("client_secret", &config.google_client_secret),
            ("redirect_uri", &redirect),
            ("grant_type", "authorization_code"),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?
        .error_for_status()
        .context("token exchange failed")?
        .json()
        .await?;

    let refresh_token = token
        .refresh_token
        .ok_or_else(|| anyhow!("no refresh_token returned (force re-consent)"))?;
    let email = gmail_email(&client, &token.access_token).await?;
    Ok(ConnectedIdentity {
        email,
        refresh_token,
    })
}

/// Exchange a stored refresh token for a fresh access token.
pub async fn google_access_token(
    config: &OAuthConfig,
    refresh_token: &str,
) -> Result<AccessToken, TokenRefreshError> {
    let response = reqwest::Client::new()
        .post(GOOGLE_TOKEN)
        .form(&[
            ("client_id", config.google_client_id.as_str()),
            ("client_secret", &config.google_client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(send_error)?;
    refresh_response(response).await
}

/// Run the Microsoft loopback + PKCE flow. Public client — no client secret;
/// PKCE secures the exchange. Returns the user's email + refresh token.
pub async fn microsoft_authorize(config: &OAuthConfig) -> anyhow::Result<ConnectedIdentity> {
    if config.microsoft_client_id.is_empty() {
        return Err(anyhow!(
            "missing Microsoft client id — set MAILAGENT_MICROSOFT_CLIENT_ID or ~/.beeline/config.toml"
        ));
    }

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    // `http://localhost` is the registered loopback URI; Entra accepts any port.
    let redirect = format!("http://localhost:{}", listener.local_addr()?.port());

    let (verifier, challenge) = pkce();
    let state = random_b64url(16);

    let auth_url = format!(
        "{MS_AUTH}?client_id={cid}&response_type=code&redirect_uri={redir}&response_mode=query\
         &scope={scope}&code_challenge={chal}&code_challenge_method=S256&state={state}\
         &prompt=select_account",
        cid = urlencode(&config.microsoft_client_id),
        redir = urlencode(&redirect),
        scope = urlencode(MS_SCOPE),
        chal = challenge,
        state = state,
    );

    eprintln!("Opening your browser to sign in to Microsoft...");
    eprintln!("If it doesn't open, paste this URL:\n{auth_url}\n");
    let _ = std::process::Command::new("open").arg(&auth_url).spawn();

    let (code, returned_state) = wait_for_redirect(&listener).await?;
    if returned_state != state {
        return Err(anyhow!("OAuth state mismatch — aborting"));
    }

    let client = reqwest::Client::new();
    let token = ms_token_request(
        &client,
        &config.microsoft_client_id,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.as_str()),
        ],
    )
    .await?;

    let refresh_token = token
        .refresh_token
        .ok_or_else(|| anyhow!("no refresh_token returned"))?;
    // Read the email from the id_token claims (we requested `openid`), avoiding
    // a Graph /me call and the extra User.Read scope it would require.
    let email = token
        .id_token
        .as_deref()
        .and_then(email_from_id_token)
        .ok_or_else(|| anyhow!("could not read email from id_token"))?;
    Ok(ConnectedIdentity {
        email,
        refresh_token,
    })
}

/// Extract the account email from an OIDC id_token's claims (no signature
/// verification needed — the token came straight from the token endpoint over
/// TLS, and we only use it to label the account).
fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;

    #[derive(Deserialize)]
    struct Claims {
        email: Option<String>,
        preferred_username: Option<String>,
        upn: Option<String>,
    }
    let claims: Claims = serde_json::from_slice(&bytes).ok()?;
    claims.email.or(claims.preferred_username).or(claims.upn)
}

/// Exchange a stored Microsoft refresh token for a fresh access token.
pub async fn microsoft_access_token(
    config: &OAuthConfig,
    refresh_token: &str,
) -> Result<AccessToken, TokenRefreshError> {
    let response = reqwest::Client::new()
        .post(MS_TOKEN)
        .form(&[
            ("client_id", config.microsoft_client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", MS_SCOPE),
        ])
        .send()
        .await
        .map_err(send_error)?;
    refresh_response(response).await
}

async fn ms_token_request(
    client: &reqwest::Client,
    client_id: &str,
    extra: &[(&str, &str)],
) -> anyhow::Result<TokenResponse> {
    let mut form: Vec<(&str, &str)> = vec![("client_id", client_id)];
    form.extend_from_slice(extra);

    let response = client.post(MS_TOKEN).form(&form).send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("microsoft token request failed ({status}): {body}"));
    }
    Ok(response.json().await?)
}

async fn gmail_email(client: &reqwest::Client, access_token: &str) -> anyhow::Result<String> {
    #[derive(Deserialize)]
    struct Profile {
        #[serde(rename = "emailAddress")]
        email_address: String,
    }
    let profile: Profile = client
        .get(GMAIL_PROFILE)
        .bearer_auth(access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(profile.email_address)
}

/// Accept connections until one carries the OAuth `code` (or `error`); reply
/// with a friendly page and return the code + state. Ignores stray requests
/// (e.g. favicon) that some browsers fire at the loopback port.
async fn wait_for_redirect(listener: &TcpListener) -> anyhow::Result<(String, String)> {
    loop {
        let (mut socket, _) = listener.accept().await?;
        let mut buf = [0u8; 4096];
        let n = socket.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);

        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("");
        let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");

        let (mut code, mut state, mut error) = (None, None, None);
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                let value = urldecode(value);
                match key {
                    "code" => code = Some(value),
                    "state" => state = Some(value),
                    "error" => error = Some(value),
                    _ => {}
                }
            }
        }

        if code.is_none() && error.is_none() {
            // Not the redirect we're waiting for — acknowledge and keep listening.
            let _ = socket
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }

        let body = "<html><body style='font-family:-apple-system,sans-serif;text-align:center;padding-top:4em'>\
                    <h2>Beeline</h2><p>You're connected. You can close this window.</p></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(response.as_bytes()).await;

        if let Some(e) = error {
            return Err(anyhow!("authorization denied: {e}"));
        }
        return Ok((
            code.ok_or_else(|| anyhow!("no code in redirect"))?,
            state.unwrap_or_default(),
        ));
    }
}

fn random_b64url(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// PKCE (verifier, S256 challenge).
fn pkce() -> (String, String) {
    let verifier = random_b64url(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Percent-encode a query component (unreserved chars per RFC 3986).
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

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 3;
                }
                Err(_) => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
