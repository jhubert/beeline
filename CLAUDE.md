# CLAUDE.md

Beeline — a local-first AI mail agent. One Rust binary that is a CLI, an MCP
server (stdio), and a control daemon; an optional Tauri GUI is a thin client.
Product name **Beeline**; published by the AppCamp studio. See `README.md` for
the user-facing overview and `docs/security.md` for the data-flow/scope details.
`SPEC.md` (gitignored, local-only) is the original design doc.

## Naming (read this first — it trips people up)
- The **binary / command is `beeline`** (`crates/cli`, `[[bin]] name = "beeline"`).
- The **crate packages are `mailagent-*`** (`mailagent-core`, `mailagent-types`,
  etc.) — internal plumbing, never user-visible. **Don't rename them.**
- On-disk data dir is **`~/.beeline/`** (db `beeline.sqlite`, `control.sock`,
  `config.toml`).
- Bundle id / Keychain service / launchd label is `com.appcamp.beelinemailagent`
  (contains "mailagent" but is a stable signed identity — leave it).

## Build / test / run
```sh
cargo build                 # builds the core workspace (NOT the desktop app)
cargo test                  # unit tests live in crates/storage/src/db.rs
cargo test -p mailagent-storage -- --ignored   # real-Keychain round-trip test
./target/debug/beeline doctor                   # health/config check
```
The desktop app is a **separate Cargo workspace** (`apps/desktop/src-tauri`) to
keep core builds lean — `cargo build` at the root does not touch it. Run it with
`cd apps/desktop && pnpm install && pnpm tauri dev`. Build the signed/notarized
DMG with `scripts/build-macos.sh`.

CLI subcommands: `doctor accounts add-account search read draft-reply reconnect
remove-account install-mcp install-cli mcp serve ctl`.

## Architecture
- `crates/types` — normalized model (camelCase JSON).
- `crates/providers` — `MailProvider` trait + real Gmail and Microsoft Graph
  adapters (search, read, create-draft / draft-reply).
- `crates/storage` — SQLite (accounts, local-ids, audit, confirmations) +
  `SecretStore` (Keychain via `keyring`).
- `crates/core` — `MailAgent` facade: OAuth (PKCE loopback), token cache, policy,
  local-id mapping, config. The single place business logic lives.
- `crates/mcp` — MCP server (hand-rolled JSON-RPC over stdio), agent-facing.
- `crates/control` — control API over a unix socket, human/GUI-facing.
- `crates/cli` — the `beeline` binary; thin shell over core.
- `apps/desktop` — Tauri GUI (separate workspace), thin client over core.

Two local interfaces, two audiences: **MCP (stdio)** for AI clients, **control
UDS** for the CLI/GUI. The binary is authoritative; the GUI is optional.

## Invariants (don't break these)
- **Read + draft only. Never send, delete, archive, or move.** Those actions are
  intentionally not exposed. This is the core safety/trust claim and is reflected
  in the privacy policy. Drafting is non-destructive, so it is *not* permission-
  gated; send/archive/etc. (if ever added) go through the `confirmations` flow.
- **OAuth scopes are minimal and must match Google's verification:** Gmail
  `gmail.readonly` + `gmail.drafts.create` (create-only, cannot send — narrower
  than `compose`); Microsoft `Mail.ReadWrite`. We get the Gmail address via
  `users.getProfile` (under readonly) so we request no `openid`/`email` scopes.
- **No servers; local-first.** Mail + tokens never leave the device via Beeline.
  Tokens live only in the macOS Keychain. This earns the client-only exemption
  from Google's CASA security assessment — keep it true.
- **`keyring` REQUIRES the `apple-native` feature** (in `crates/storage`) — without
  a backend feature it silently uses an in-memory mock that loses tokens. (Add
  `windows-native` / `sync-secret-service` when porting.)
- OAuth client config resolves: runtime env (`MAILAGENT_*`) → `~/.beeline/config.toml`
  → compile-time `option_env!` (release builds bake it in via `build-macos.sh`).
  `config.sh`/`config.toml` are gitignored — never commit secrets.

## Working conventions
- **Never `git add -A`** — it has twice swept in large stray files (a .psd, an
  .mp4). Use targeted `git add <paths>`. `*.psd`, `*.mp4`, `config.sh`, `SPEC.md`,
  `~/.beeline` data, and `target/` are gitignored.
- Feature work goes on a branch, then `git merge --ff-only main` + push. Commit
  messages end with the `Co-Authored-By` trailer.
- The **AppCamp website is a separate repo** at `~/src/AppCamp/website` (no git
  remote; deploys via its own `deploy.sh`). The Beeline landing page and the
  privacy policy (with the Google Limited Use disclosure) live there — keep them
  consistent with the app's actual scopes/behavior.
- `TODO.md` tracks the road to launch (clean-machine test + Google verification
  are the remaining blockers).
