//! SQLite store (SPEC.md §10.2). Holds account records, local-id mappings, and
//! audit logs — never message bodies or tokens. A single connection guarded by
//! a mutex makes `Db` `Sync` so it can be shared behind an `Arc`.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use mailagent_types::{AccountStatus, ConnectedAccount, Permissions, Provider};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::{AuditEvent, Confirmation, ProviderRef};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id               TEXT PRIMARY KEY,
    provider         TEXT NOT NULL,
    alias            TEXT NOT NULL,
    email            TEXT NOT NULL,
    display_name     TEXT,
    enabled          INTEGER NOT NULL DEFAULT 1,
    perm_read        INTEGER NOT NULL DEFAULT 1,
    perm_modify      INTEGER NOT NULL DEFAULT 0,
    perm_send        INTEGER NOT NULL DEFAULT 0,
    perm_attachments INTEGER NOT NULL DEFAULT 0,
    status           TEXT NOT NULL DEFAULT 'connected',
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

-- local_msg_<seq> / local_thread_<seq> → (provider, account, provider id).
-- `seq` is the rowid; the local id embeds it. Unique per origin so repeated
-- searches reuse the same local id rather than minting duplicates.
CREATE TABLE IF NOT EXISTS local_ids (
    seq         INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL,
    provider    TEXT NOT NULL,
    account_id  TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(kind, account_id, provider_id)
);

-- Mutation events only; never message bodies (SPEC.md §17.5, §19).
CREATE TABLE IF NOT EXISTS audit_log (
    id             INTEGER PRIMARY KEY,
    at             TEXT NOT NULL DEFAULT (datetime('now')),
    account_alias  TEXT,
    provider       TEXT,
    action         TEXT NOT NULL,
    success        INTEGER NOT NULL,
    error_category TEXT
);

-- Pending mutations awaiting user approval (SPEC.md §12.2). status is
-- pending|approved|denied. `detail` holds a metadata-only JSON summary.
CREATE TABLE IF NOT EXISTS confirmations (
    id          INTEGER PRIMARY KEY,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    account_id  TEXT,
    action      TEXT NOT NULL,
    detail      TEXT,
    status      TEXT NOT NULL DEFAULT 'pending',
    resolved_at TEXT
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening database at {}", path.display()))?;
        Self::from_conn(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> anyhow::Result<Self> {
        let db = Self {
            conn: Mutex::new(conn),
        };
        {
            let conn = db.conn.lock().unwrap();
            // WAL lets the `serve` daemon and the MCP process share the file
            // (concurrent readers + one writer); busy_timeout absorbs contention.
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
                .context("configuring sqlite pragmas")?;
            conn.execute_batch(SCHEMA)
                .context("running schema migration")?;
        }
        Ok(db)
    }

    // --- accounts (SPEC.md §8.1) ---------------------------------------------

    pub fn list_accounts(&self) -> anyhow::Result<Vec<ConnectedAccount>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(ACCOUNT_SELECT)?;
        let rows = stmt.query_map([], row_to_account)?;
        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row?.into_account()?);
        }
        Ok(accounts)
    }

    pub fn get_account(&self, id: &str) -> anyhow::Result<Option<ConnectedAccount>> {
        let conn = self.conn.lock().unwrap();
        let raw = conn
            .query_row(
                &format!("{ACCOUNT_SELECT} WHERE id = ?1"),
                params![id],
                row_to_account,
            )
            .optional()?;
        raw.map(RawAccount::into_account).transpose()
    }

    pub fn upsert_account(&self, account: &ConnectedAccount) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts
                (id, provider, alias, email, display_name, enabled,
                 perm_read, perm_modify, perm_send, perm_attachments, status, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
                provider=excluded.provider, alias=excluded.alias, email=excluded.email,
                display_name=excluded.display_name, enabled=excluded.enabled,
                perm_read=excluded.perm_read, perm_modify=excluded.perm_modify,
                perm_send=excluded.perm_send, perm_attachments=excluded.perm_attachments,
                status=excluded.status, updated_at=datetime('now')",
            params![
                account.id,
                account.provider.as_str(),
                account.alias,
                account.email,
                account.display_name,
                account.enabled,
                account.permissions.read,
                account.permissions.modify,
                account.permissions.send,
                account.permissions.attachments,
                account.status.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn set_account_status(&self, id: &str, status: AccountStatus) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE accounts SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![status.as_str(), id],
        )?;
        Ok(changed > 0)
    }

    /// Delete an account and its local-id mappings. Returns false if no such
    /// account existed. (Keychain token removal is the caller's job.)
    pub fn delete_account(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM local_ids WHERE account_id = ?1", params![id])?;
        let removed = conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }

    /// Idempotent dev seed (used by tests). No longer auto-run on open — real
    /// accounts arrive via the OAuth add-account flow.
    pub fn seed_demo_accounts_if_empty(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        if count > 0 {
            return Ok(());
        }
        let demo = [
            ("acct_personal", "gmail", "personal", "you@gmail.com"),
            ("acct_work", "microsoft", "work", "you@company.com"),
        ];
        for (id, provider, alias, email) in demo {
            conn.execute(
                "INSERT INTO accounts (id, provider, alias, email, perm_read, status)
                 VALUES (?1, ?2, ?3, ?4, 1, 'connected')",
                params![id, provider, alias, email],
            )?;
        }
        Ok(())
    }

    // --- local id mapping (SPEC.md §9) ---------------------------------------

    pub fn mint_local_id(&self, reference: &ProviderRef) -> anyhow::Result<String> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT seq FROM local_ids WHERE kind='msg' AND account_id=?1 AND provider_id=?2",
                params![reference.account_id, reference.provider_message_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(seq) = existing {
            return Ok(format!("local_msg_{seq}"));
        }
        conn.execute(
            "INSERT INTO local_ids (kind, provider, account_id, provider_id)
             VALUES ('msg', ?1, ?2, ?3)",
            params![
                reference.provider.as_str(),
                reference.account_id,
                reference.provider_message_id
            ],
        )?;
        Ok(format!("local_msg_{}", conn.last_insert_rowid()))
    }

    pub fn resolve_local_id(&self, local_id: &str) -> anyhow::Result<Option<ProviderRef>> {
        let Some(seq) = local_id
            .strip_prefix("local_msg_")
            .and_then(|s| s.parse::<i64>().ok())
        else {
            return Ok(None);
        };
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT provider, account_id, provider_id FROM local_ids WHERE kind='msg' AND seq=?1",
                params![seq],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.and_then(|(provider, account_id, provider_message_id)| {
            Provider::from_str(&provider).map(|provider| ProviderRef {
                provider,
                account_id,
                provider_message_id,
            })
        }))
    }

    // --- audit log (SPEC.md §17.5) -------------------------------------------

    pub fn record_audit(
        &self,
        account_alias: Option<&str>,
        provider: Option<Provider>,
        action: &str,
        success: bool,
        error_category: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO audit_log (account_alias, provider, action, success, error_category)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                account_alias,
                provider.map(|p| p.as_str()),
                action,
                success,
                error_category
            ],
        )?;
        Ok(())
    }

    pub fn recent_audit(&self, limit: u32) -> anyhow::Result<Vec<AuditEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, at, account_alias, provider, action, success, error_category
             FROM audit_log ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(AuditEvent {
                id: row.get(0)?,
                at: row.get(1)?,
                account_alias: row.get(2)?,
                provider: row.get(3)?,
                action: row.get(4)?,
                success: row.get(5)?,
                error_category: row.get(6)?,
            })
        })?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    // --- confirmations (SPEC.md §12.2) ---------------------------------------

    pub fn create_confirmation(
        &self,
        account_id: Option<&str>,
        action: &str,
        detail: Option<&str>,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO confirmations (account_id, action, detail) VALUES (?1, ?2, ?3)",
            params![account_id, action, detail],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_confirmations(&self, status: &str) -> anyhow::Result<Vec<Confirmation>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, created_at, account_id, action, detail, status
             FROM confirmations WHERE status = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![status], |row| {
            Ok(Confirmation {
                id: row.get(0)?,
                created_at: row.get(1)?,
                account_id: row.get(2)?,
                action: row.get(3)?,
                detail: row.get(4)?,
                status: row.get(5)?,
            })
        })?;
        let mut confirmations = Vec::new();
        for row in rows {
            confirmations.push(row?);
        }
        Ok(confirmations)
    }

    /// Resolve a pending confirmation. Returns false if it was already resolved
    /// or never existed (so a double-approve can't fire an action twice).
    pub fn resolve_confirmation(&self, id: i64, approve: bool) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let new_status = if approve { "approved" } else { "denied" };
        let changed = conn.execute(
            "UPDATE confirmations SET status = ?1, resolved_at = datetime('now')
             WHERE id = ?2 AND status = 'pending'",
            params![new_status, id],
        )?;
        Ok(changed > 0)
    }
}

const ACCOUNT_SELECT: &str = "SELECT id, provider, alias, email, display_name, enabled, \
     perm_read, perm_modify, perm_send, perm_attachments, status FROM accounts";

/// Raw column tuple, parsed into a `ConnectedAccount` outside the rusqlite
/// closure so provider/status parse errors surface as `anyhow` errors.
struct RawAccount {
    id: String,
    provider: String,
    alias: String,
    email: String,
    display_name: Option<String>,
    enabled: bool,
    perm_read: bool,
    perm_modify: bool,
    perm_send: bool,
    perm_attachments: bool,
    status: String,
}

impl RawAccount {
    fn into_account(self) -> anyhow::Result<ConnectedAccount> {
        Ok(ConnectedAccount {
            provider: Provider::from_str(&self.provider)
                .with_context(|| format!("unknown provider '{}'", self.provider))?,
            status: AccountStatus::from_str(&self.status)
                .with_context(|| format!("unknown status '{}'", self.status))?,
            id: self.id,
            alias: self.alias,
            email: self.email,
            display_name: self.display_name,
            enabled: self.enabled,
            permissions: Permissions {
                read: self.perm_read,
                modify: self.perm_modify,
                send: self.perm_send,
                attachments: self.perm_attachments,
            },
        })
    }
}

fn row_to_account(row: &Row) -> rusqlite::Result<RawAccount> {
    Ok(RawAccount {
        id: row.get(0)?,
        provider: row.get(1)?,
        alias: row.get(2)?,
        email: row.get(3)?,
        display_name: row.get(4)?,
        enabled: row.get(5)?,
        perm_read: row.get(6)?,
        perm_modify: row.get(7)?,
        perm_send: row.get(8)?,
        perm_attachments: row.get(9)?,
        status: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_id_roundtrip_and_reuse() {
        let db = Db::open_in_memory().unwrap();
        let reference = ProviderRef {
            provider: Provider::Gmail,
            account_id: "acct_personal".into(),
            provider_message_id: "gmsg_1".into(),
        };

        let id = db.mint_local_id(&reference).unwrap();
        assert!(id.starts_with("local_msg_"));

        // Same origin mints the same local id (no duplicates).
        assert_eq!(id, db.mint_local_id(&reference).unwrap());

        // Round-trips back to the original provider-scoped reference.
        let resolved = db.resolve_local_id(&id).unwrap().expect("should resolve");
        assert_eq!(resolved, reference);

        // Unknown / malformed ids resolve to None, not an error.
        assert!(db.resolve_local_id("local_msg_999999").unwrap().is_none());
        assert!(db.resolve_local_id("not-a-local-id").unwrap().is_none());
    }

    #[test]
    fn distinct_origins_get_distinct_ids() {
        let db = Db::open_in_memory().unwrap();
        let a = db
            .mint_local_id(&ProviderRef {
                provider: Provider::Gmail,
                account_id: "acct_personal".into(),
                provider_message_id: "gmsg_1".into(),
            })
            .unwrap();
        let b = db
            .mint_local_id(&ProviderRef {
                provider: Provider::Microsoft,
                account_id: "acct_work".into(),
                provider_message_id: "gmsg_1".into(), // same provider id, different account
            })
            .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn confirmation_lifecycle() {
        let db = Db::open_in_memory().unwrap();

        // No pending confirmations to start.
        assert!(db.list_confirmations("pending").unwrap().is_empty());

        let id = db
            .create_confirmation(Some("acct_personal"), "send", Some("{\"draft\":\"local_draft_1\"}"))
            .unwrap();

        let pending = db.list_confirmations("pending").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].action, "send");

        // Approving moves it out of pending.
        assert!(db.resolve_confirmation(id, true).unwrap());
        assert!(db.list_confirmations("pending").unwrap().is_empty());
        assert_eq!(db.list_confirmations("approved").unwrap().len(), 1);

        // Re-resolving a settled confirmation is a no-op (can't fire twice).
        assert!(!db.resolve_confirmation(id, true).unwrap());
        // Resolving an id that never existed is a no-op, not an error.
        assert!(!db.resolve_confirmation(424242, true).unwrap());
    }

    #[test]
    fn permission_change_persists_and_audits() {
        let db = Db::open_in_memory().unwrap();
        db.seed_demo_accounts_if_empty().unwrap();

        let mut account = db.get_account("acct_personal").unwrap().unwrap();
        assert!(!account.permissions.modify);
        account.permissions.modify = true;
        db.upsert_account(&account).unwrap();
        db.record_audit(Some("personal"), None, "permissions_changed", true, None)
            .unwrap();

        let reloaded = db.get_account("acct_personal").unwrap().unwrap();
        assert!(reloaded.permissions.modify);
        assert_eq!(db.recent_audit(10).unwrap().len(), 1);
    }

    #[test]
    fn account_status_updates_and_persists() {
        let db = Db::open_in_memory().unwrap();
        db.seed_demo_accounts_if_empty().unwrap();

        assert!(db
            .set_account_status("acct_personal", AccountStatus::NeedsReconnect)
            .unwrap());
        let account = db.get_account("acct_personal").unwrap().unwrap();
        assert_eq!(account.status, AccountStatus::NeedsReconnect);

        // Reconnecting clears it.
        db.set_account_status("acct_personal", AccountStatus::Connected)
            .unwrap();
        assert_eq!(
            db.get_account("acct_personal").unwrap().unwrap().status,
            AccountStatus::Connected
        );

        // Unknown id → false, not an error.
        assert!(!db
            .set_account_status("nope", AccountStatus::Connected)
            .unwrap());
    }

    #[test]
    fn seeding_is_idempotent() {
        let db = Db::open_in_memory().unwrap();
        db.seed_demo_accounts_if_empty().unwrap();
        db.seed_demo_accounts_if_empty().unwrap();
        let accounts = db.list_accounts().unwrap();
        assert_eq!(accounts.len(), 2);
        assert!(accounts.iter().all(|a| a.permissions.read));
        assert!(accounts.iter().all(|a| !a.permissions.send));
    }
}
