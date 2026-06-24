//! OAuth client configuration (SPEC.md §11.3).
//!
//! Resolution order per value: environment variable first (handy for
//! `source config.sh` during development), then `~/.beeline/config.toml`.
//! The file matters because GUI- and launchd-spawned processes (e.g. the
//! `mcp` server Claude Desktop launches) do NOT inherit your shell env — they
//! can only see the file.
//!
//! Nothing here is committed: client IDs are public, and the Google desktop
//! "client secret" ships with installed apps and is not truly confidential, but
//! we keep all of it out of version control regardless.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub google_client_id: String,
    pub google_client_secret: String,
    pub microsoft_client_id: String,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    oauth: OAuthFile,
}

#[derive(Debug, Default, Deserialize)]
struct OAuthFile {
    google_client_id: Option<String>,
    google_client_secret: Option<String>,
    microsoft_client_id: Option<String>,
}

// Compile-time fallbacks the release build bakes in (see scripts/build-macos.sh),
// so a distributed app works with no local config. Empty in a plain dev build —
// option_env! is None unless the var was set at compile time — so dev keeps
// using runtime env / config.toml exactly as before. Client IDs are public; the
// Google desktop "secret" is non-confidential for installed apps, and injecting
// at build time keeps it out of the (public) repo.
const EMBEDDED_GOOGLE_CLIENT_ID: Option<&str> = option_env!("MAILAGENT_GOOGLE_CLIENT_ID");
const EMBEDDED_GOOGLE_CLIENT_SECRET: Option<&str> = option_env!("MAILAGENT_GOOGLE_CLIENT_SECRET");
const EMBEDDED_MICROSOFT_CLIENT_ID: Option<&str> = option_env!("MAILAGENT_MICROSOFT_CLIENT_ID");

impl OAuthConfig {
    pub fn load() -> anyhow::Result<Self> {
        let file = read_file().unwrap_or_default();

        // Runtime env wins (dev override), then ~/.beeline/config.toml, then
        // the value baked in at build time.
        let pick = |env_key: &str, file_val: Option<String>, embedded: Option<&str>| -> Option<String> {
            std::env::var(env_key)
                .ok()
                .filter(|s| !s.is_empty())
                .or(file_val)
                .or_else(|| embedded.map(str::to_string).filter(|s| !s.is_empty()))
        };

        let google_client_id = pick(
            "MAILAGENT_GOOGLE_CLIENT_ID",
            file.oauth.google_client_id,
            EMBEDDED_GOOGLE_CLIENT_ID,
        )
        .ok_or_else(|| missing("Google client id", "MAILAGENT_GOOGLE_CLIENT_ID"))?;
        let google_client_secret = pick(
            "MAILAGENT_GOOGLE_CLIENT_SECRET",
            file.oauth.google_client_secret,
            EMBEDDED_GOOGLE_CLIENT_SECRET,
        )
        .ok_or_else(|| missing("Google client secret", "MAILAGENT_GOOGLE_CLIENT_SECRET"))?;
        let microsoft_client_id = pick(
            "MAILAGENT_MICROSOFT_CLIENT_ID",
            file.oauth.microsoft_client_id,
            EMBEDDED_MICROSOFT_CLIENT_ID,
        )
        .unwrap_or_default();

        Ok(Self {
            google_client_id,
            google_client_secret,
            microsoft_client_id,
        })
    }

    /// `~/.beeline/config.toml`.
    pub fn config_path() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".beeline").join("config.toml"))
    }
}

fn read_file() -> Option<FileConfig> {
    let path = OAuthConfig::config_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

fn missing(what: &str, env_key: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "missing {what} — set {env_key} or add it to ~/.beeline/config.toml (see config.example.toml)"
    )
}
