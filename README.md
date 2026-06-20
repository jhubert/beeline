# mailagent

Local-first AI mail agent. A single distributable binary that exposes mail
tools to AI clients over MCP, keeping OAuth tokens and email contents on the
user's machine. See [SPEC.md](./SPEC.md) for the full product spec.

## Architecture

The **binary is the product**; the GUI is an optional usability client.

```
AI clients ──(MCP, stdio)──┐
                           ├──▶  mailagent (binary)  ──▶ SQLite + Keychain
GUI / CLI ──(control UDS)──┘     core: accounts/auth/
                                 policy/storage/audit/local-ids
                                        │
                                 provider adapters: Gmail · Microsoft Graph
```

Two local interfaces, two audiences: **MCP over stdio** (agent-facing,
permission-gated) and a **control API over a unix-domain socket** (human-facing,
driven by the CLI and the optional GUI). A `launchd` login-item keeps the
daemon alive independent of the GUI (daemon model B).

## Workspace layout

| Crate                  | Role                                                        |
|------------------------|-------------------------------------------------------------|
| `crates/types`         | Normalized data model (SPEC §8), camelCase JSON             |
| `crates/providers`     | `MailProvider` trait (SPEC §7) + Gmail/Graph adapters       |
| `crates/core`          | `MailAgent` facade, local-id map (§9), policy (§12)         |
| `crates/mcp`           | MCP server: JSON-RPC 2.0 over stdio                         |
| `crates/cli`           | `mailagent` binary: doctor / accounts / mcp / serve         |

## Status: Phase 0 skeleton

Providers return **stubbed data** so the MCP read path runs end-to-end before
any OAuth client registration exists. Read-only tools are wired:
`mail_list_accounts`, `mail_search`, `mail_get_message`.

Not yet implemented: real Gmail/Graph HTTP, OAuth, Keychain, SQLite, the
control-API daemon, draft/mutating tools, and the confirmation flow.

## Build & run

```sh
cargo build
cargo run -p mailagent-cli -- doctor
cargo run -p mailagent-cli -- accounts

# Smoke-test the MCP server over stdio:
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mail_search","arguments":{"account":"all","query":"agenda"}}}' \
  | cargo run -q -p mailagent-cli -- mcp
```

## Prerequisites for real functionality

1. Microsoft Entra **public client** app registration.
2. Google OAuth **desktop client** (+ start restricted-scope verification early).
