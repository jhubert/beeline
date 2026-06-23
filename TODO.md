# Beeline — road to a usable alpha

The core works: real Gmail + Outlook (read), OAuth with tokens in the Keychain, a
GUI for onboarding, and a signed + notarized DMG. This is what's left before
Beeline is genuinely ready to hand to non-technical friends.

Target: **friends-and-family alpha** (Google in Testing mode, ≤100 hand-added
test users).

---

## 🚧 Blockers — a real user can't succeed without these

- [x] **Ship the OAuth client IDs inside the app.** Done: `config.rs` now falls
  back to compile-time `option_env!` values (resolution: runtime env → config.toml
  → embedded). `scripts/build-macos.sh` sources `config.sh` and bakes the three
  `MAILAGENT_*` vars into the release build; `crates/core/build.rs` keeps them
  fresh. Secrets stay out of the public repo (injected at build, not committed);
  `mailagent doctor` reports whether config is present. Verified in a clean env.

- [ ] **Test the signed DMG on a clean Mac / second macOS user account.** The dev
  machine has `config.toml`, dev Keychain entries, and test-user access a real
  user won't. Install the actual `.dmg` as a fresh user and run the whole flow
  (add Gmail, add Outlook, search). This is the real validation of the config
  fix above and of first-run.

- [ ] **Configure the Google consent screen + add testers.** Set the privacy
  policy URL (`https://appcamp.com/privacy/` — live), app name, logo, support
  email, and app domain. Add each alpha user's Google address as a **Test user**
  (Testing mode). Their Gmail cannot connect otherwise.
  - Heads-up: Testing-mode refresh tokens expire ~weekly, so users will hit
    "needs reconnect" every few days. Reconnect is built — just set expectations.

- [ ] **Decide & build the non-technical "connect your AI" path.** We removed the
  GUI's Connect-to-Claude button (neutral positioning), but a non-technical user
  can't hand-edit Claude's config JSON or run the CLI `install-mcp`. Pick one:
  - (a) an optional, neutrally-framed "Set up with an AI assistant" button in the
    app that writes the client config (user-initiated, not marketed), or
  - (b) a friendly step-by-step help page they can follow.
  Without this, users can install Beeline but can't actually use it.

---

## ⚠️ Should-do for a solid first impression

- [ ] **Cache access tokens in memory (with expiry).** Every search/read currently
  does a token-refresh round-trip — wasteful and risks provider rate limits.
- [ ] **Cross-account query normalization (SPEC §14.2).** `newer_than:7d` is Gmail
  syntax; Graph treats it as plain text. Translate normalized query fields per
  provider so an AI gets consistent results across accounts.
- [ ] **Concurrent cross-account search.** Sequential today; parallelize so
  "search all" is fast with several accounts.
- [ ] **GUI: per-account permission toggles.** Backend (`set_account_permissions`)
  is ready; surface read/draft/etc. per account.
- [ ] **Friendlier GUI errors** for `admin_approval_required`,
  `provider_unavailable`, and rate limiting.
- [ ] **A short "Getting started" for users** (install → add account → connect AI),
  tied to the AI-connection decision above.

---

## 🔭 Post-alpha / later

- [ ] **Phase 3 mutating tools** — draft reply / send / archive / mark-read, gated
  by the confirmation flow (the `confirmations` table + control-API resolve are
  already built). This is the "draft replies" value; it requires the `serve`
  daemon + launchd login-item to actually run so confirmations can be surfaced.
- [ ] **launchd login-item** for the `serve` daemon (only needed once mutations /
  confirmations ship — the read-only alpha doesn't need it).
- [ ] **Attachment download** with explicit confirmation.
- [ ] **Auto-update** (Tauri updater, or Sparkle like Legal Message Export).
- [ ] **Google verification beyond Testing mode** — brand verification + Limited
  Use attestation. The client-only, serverless architecture should exempt us from
  the full third-party CASA security assessment. Needed to go past 100 users.
- [ ] **Microsoft publisher verification** — reduces the admin-consent prompt on
  some tenants.
- [ ] **More providers** (IMAP / iCloud) behind the existing `MailProvider` trait.
- [ ] **Provider test matrix:** Outlook.com consumer, Microsoft 365 work, Gmail
  consumer, Google Workspace.

---

## ✅ Done

- Rust workspace; single `mailagent` binary = CLI + MCP server + control daemon
- Real Gmail + Microsoft Graph (search, read), normalized; OAuth + PKCE; tokens
  in the macOS Keychain; SQLite store; local-id indirection
- MCP tools: `mail_list_accounts`, `mail_search`, `mail_get_message`
- CLI: doctor, accounts, add-account, search, read, reconnect, remove-account,
  install-mcp, mcp, serve, ctl
- Tauri GUI: account onboarding (add / remove / reconnect), status auto-refresh
- Reconnect flow for expired/revoked tokens
- Signed + notarized + stapled DMG (verified accepted by Gatekeeper)
- Privacy policy + data-flow doc + landing page; neutral local-tool positioning
