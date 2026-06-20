//! Secret storage abstraction (SPEC.md §10.1, §19). OAuth tokens live in the OS
//! secret store, never in SQLite or plaintext files. The trait keeps the call
//! site backend-agnostic so a Windows Credential Manager backend slots in later
//! and tests can use an in-memory store.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Context;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> anyhow::Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> anyhow::Result<()>;
    fn delete(&self, key: &str) -> anyhow::Result<()>;
}

/// Backed by the `keyring` crate: macOS Keychain today, Windows Credential
/// Manager / Linux secret-service on those targets with no call-site change.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let entry = keyring::Entry::new(&self.service, key)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e).context("reading secret from keyring"),
        }
    }

    fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        keyring::Entry::new(&self.service, key)?
            .set_password(value)
            .context("writing secret to keyring")
    }

    fn delete(&self, key: &str) -> anyhow::Result<()> {
        let entry = keyring::Entry::new(&self.service, key)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e).context("deleting secret from keyring"),
        }
    }
}

/// In-memory store for tests and headless CI — never persists.
#[derive(Default)]
pub struct MemorySecretStore {
    map: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemorySecretStore {
    fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real macOS Keychain round-trip. Ignored by default because it touches
    /// the OS keychain (and may prompt). Run manually on macOS:
    ///
    ///   cargo test -p mailagent-storage -- --ignored
    ///
    /// This guards against the keyring "silent mock store" regression: a same
    /// process get() would still pass under the mock, so we verify with Apple's
    /// signed `security` tool that the secret actually landed in the real
    /// keychain — which the mock would fail.
    #[test]
    #[ignore = "touches the real macOS Keychain; run with --ignored"]
    fn keychain_real_roundtrip() {
        let service = "com.appcamp.beelinemailagent.selftest";
        let key = "roundtrip-test-account";
        let value = "secret-value-123";

        let store = KeyringStore::new(service);
        store.set(key, value).unwrap();
        assert_eq!(store.get(key).unwrap().as_deref(), Some(value));

        // Confirm via the OS, not our own (possibly-mock) code path.
        let output = std::process::Command::new("security")
            .args(["find-generic-password", "-s", service, "-a", key, "-w"])
            .output()
            .expect("run `security`");
        let found = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            found.trim(),
            value,
            "secret not in the real keychain — keyring may be using the mock store"
        );

        store.delete(key).unwrap();
        assert!(store.get(key).unwrap().is_none());
    }
}
