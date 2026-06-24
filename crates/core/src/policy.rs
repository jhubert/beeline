//! Permission tiers + default action policy (SPEC.md §12). This is the layer
//! that makes the product different from a raw provider wrapper, so it lives in
//! core where the CLI, MCP server, and GUI all route through it.
//!
//! Phase 0 implements the read gate only. Draft-first enforcement, the
//! confirmation-as-core-state flow (send/archive/attachments), and per-client
//! grants land in Phase 1/3.

use mailagent_types::ConnectedAccount;

pub fn can_read(account: &ConnectedAccount) -> bool {
    account.enabled && account.permissions.read
}

// Drafting is intentionally ungated — a draft is non-destructive. The
// permission tiers below exist for the actions that are: send / archive / move.

/// Actions that always require explicit user confirmation (SPEC.md §12.2),
/// regardless of granted permissions. Wired into tool dispatch in Phase 3.
pub fn requires_confirmation(action: &str) -> bool {
    matches!(
        action,
        "send" | "archive" | "move" | "download_attachment" | "bulk"
    )
}
