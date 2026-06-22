# Beeline desktop app

Tauri GUI over `mailagent-core` — onboarding, account management, and one-click
Claude Desktop setup for non-technical users. The GUI holds no logic; it calls
the same core facade as the CLI and MCP server.

## Run in dev

From `apps/desktop/`:

```sh
pnpm install                 # installs the Tauri CLI (@tauri-apps/cli)

# Point the GUI at the dev mailagent binary (used for "Connect to Claude Desktop").
# Build it first from the repo root: cargo build
export BEELINE_MCP_BIN="$(cd ../.. && pwd)/target/debug/mailagent"

pnpm tauri dev
```

OAuth client creds are read from `~/.mailagent/config.toml` (or env), the same
as the CLI. Accounts, tokens (Keychain), and the SQLite store are shared with
the CLI/MCP server, so anything added here shows up there and vice versa.

## Build a signed, notarized DMG

Generate icons once from the logo, then build (see `scripts/` and the bundle
config in `src-tauri/tauri.conf.json`):

```sh
pnpm tauri icon ../../assets/logo-icon.png
../../scripts/build-macos.sh
```

## Status

v0 scaffold. Backend commands wrap the tested core API; the Tauri wiring and
frontend need a real `pnpm tauri dev` run to verify (and then iterate).
