//! OAuth client configuration (SPEC.md §11.3).
//!
//! Resolution order per value: environment variable first (handy for
//! `source config.sh` during development), then `~/.mailagent/config.toml`.
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

impl OAuthConfig {
    pub fn load() -> anyhow::Result<Self> {
        let file = read_file().unwrap_or_default();

        // env wins; fall back to the file value.
        let pick = |env_key: &str, file_val: Option<String>| -> Option<String> {
            std::env::var(env_key)
                .ok()
                .filter(|s| !s.is_empty())
                .or(file_val)
        };

        let google_client_id = pick("MAILAGENT_GOOGLE_CLIENT_ID", file.oauth.google_client_id)
            .ok_or_else(|| missing("Google client id", "MAILAGENT_GOOGLE_CLIENT_ID"))?;
        let google_client_secret =
            pick("MAILAGENT_GOOGLE_CLIENT_SECRET", file.oauth.google_client_secret)
                .ok_or_else(|| missing("Google client secret", "MAILAGENT_GOOGLE_CLIENT_SECRET"))?;
        let microsoft_client_id =
            pick("MAILAGENT_MICROSOFT_CLIENT_ID", file.oauth.microsoft_client_id)
                .unwrap_or_default();

        Ok(Self {
            google_client_id,
            google_client_secret,
            microsoft_client_id,
        })
    }

    /// `~/.mailagent/config.toml`.
    pub fn config_path() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".mailagent").join("config.toml"))
    }
}

fn read_file() -> Option<FileConfig> {
    let path = OAuthConfig::config_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

fn missing(what: &str, env_key: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "missing {what} — set {env_key} or add it to ~/.mailagent/config.toml (see config.example.toml)"
    )
}
