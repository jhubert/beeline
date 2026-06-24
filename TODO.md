# Beeline — road to launch

The core works: real Gmail + Outlook (search, read, **draft**), OAuth with tokens
in the Keychain, a GUI for onboarding, and a signed + notarized DMG that embeds
its own config. This tracks what's left.

Target: **public launch** — Google app verified for production (not Testing
mode). We expect to qualify for the **client-only exemption** from the third-party
CASA security assessment (no servers, nothing transmitted off-device), but still
go through Google's verification *review*.

---

## 🚧 Blockers — before real users

- [x] **Ship OAuth client config inside the app.** `config.rs` falls back to
  compile-time `option_env!` values; `build-macos.sh` bakes them into the release
  (secrets injected at build, not committed). Verified in a clean env.

- [x] **Draft creation (Gmail + Outlook), ungated.** A draft is non-destructive,
  so it isn't gated; send/archive/delete are simply not exposed. Scopes:
  `gmail.compose`, `Mail.ReadWrite`.

- [x] **AI-connection path — decided.** No in-app integration; Claude Code runs
  the CLI directly, Claude Desktop uses `mailagent install-mcp`.

- [ ] **Clean-machine test of the signed DMG.** Install the actual `.dmg` as a
  fresh macOS user (no `config.toml`, no dev Keychain), then add a Gmail + an
  Outlook account and search/draft. Real validation of first-run + embedded config.

- [ ] **Google production verification.** The big one:
  1. **Re-deploy the AppCamp site** — privacy policy + landing now say "reads and
     drafts, never sends/deletes" (they must match the live scopes for review).
  2. **Verify `appcamp.com`** ownership in Google Search Console.
  3. **Data access scopes:** declare `gmail.readonly` + `gmail.compose`.
  4. **Demo video** showing the OAuth consent + each scope's use (search/read =
     readonly; create a draft = compose).
  5. Submit; respond to the reviewer; complete any client-only self-attestation.
  - Likely reviewer questions: why `gmail.compose` (answer: minimal scope for
    drafts; no send tool) and the AI-transfer angle (answer: user-directed; Beeline
    transmits nothing). Verification removes the 100-user cap + ~7-day token expiry.

- [x] **Microsoft:** `Mail.ReadWrite` (delegated) added to the Entra app's API
  permissions.

---

## ⚠️ Should-do for a solid first impression

- [x] **Cache access tokens in memory (with expiry).** Done: tokens cached per
  account until ~60s before expiry; invalidated on reconnect / removal /
  needs_reconnect. No more refresh on every call.
- [x] **Cross-account query normalization (SPEC §14.2).** Done: `mail_search` /
  CLI expose structured fields (from, to, subject, since, before); providers
  translate each per their syntax, so the agent avoids provider-specific operators.
- [x] **Concurrent cross-account search.** Done: per-account token+search runs via
  `join_all`; results merged and sorted newest-first.
- [ ] **Friendlier GUI errors** for `admin_approval_required`,
  `provider_unavailable`, and rate limiting.
- [ ] **"Using Beeline with Claude" guide** — teach the AI to drive the CLI
  (command reference + example prompts) + the `install-mcp` one-liner.

---

## 🔭 Post-launch / later

- [ ] **Send / archive / mark-read** — the remaining mutating tools, gated by the
  confirmation flow (`confirmations` table + control-API resolve already built).
  These need the `serve` daemon running so confirmations can be surfaced.
  (Draft creation, the draft-first core, is done.)
- [ ] **launchd login-item** for the `serve` daemon (needed once send/archive ship).
- [ ] **GUI: per-account permission toggles** — for the send/archive tiers above
  (drafting is ungated, so no toggle needed for it).
- [ ] **Attachment download** with explicit confirmation.
- [ ] **Auto-update** (Tauri updater, or Sparkle like Legal Message Export).
- [ ] **Microsoft publisher verification** — reduces the admin-consent prompt on
  some tenants.
- [ ] **More providers** (IMAP / iCloud) behind the existing `MailProvider` trait.
- [ ] **Provider test matrix:** Outlook.com consumer, Microsoft 365 work, Gmail
  consumer, Google Workspace.
- [ ] **Rich-text (HTML) draft bodies** — currently plain text (a fine default).

---

## ✅ Done

- Rust workspace; single `mailagent` binary = CLI + MCP server + control daemon
- Real Gmail + Microsoft Graph: search, read, **draft + draft-reply** (threaded);
  normalized; OAuth + PKCE; tokens in macOS Keychain; SQLite; local-id indirection
- OAuth client config embedded in release builds (works with no local config)
- MCP tools: list_accounts, search, get_message, create_draft, create_draft_reply
- CLI: doctor, accounts, add-account, search, read, draft-reply, reconnect,
  remove-account, install-mcp, mcp, serve, ctl
- Tauri GUI: onboarding (add / remove / reconnect), status auto-refresh
- Reconnect flow for expired/revoked tokens (also upgrades scope)
- Signed + notarized + stapled DMG (Gatekeeper-accepted)
- Privacy policy + data-flow doc + landing page; neutral local-tool positioning
  ("reads and drafts, never sends or deletes")
