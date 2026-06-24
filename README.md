# Beeline

Beeline gives you local, programmatic access to your own email. It runs
**entirely on your Mac** — it has no servers and **never transmits your mail
anywhere**. Connect it to your own tools (an AI assistant via MCP, or scripts
via the CLI); what you do with your mail is up to you.

A single `beeline` binary is the product: a CLI, a local [MCP](https://modelcontextprotocol.io)
server, and (optionally) a small desktop app for managing accounts. Tokens live
in the macOS Keychain; account metadata in a local SQLite store.

## What it is — and isn't

- **It is** a local capability layer for your mailboxes: search, read, and draft
  across Gmail and Outlook/Microsoft 365 behind one interface.
- **It isn't** a hosted service, a mail client, or an AI product. Beeline does
  not bundle, endorse, or route your mail to any AI on its own. If *you* connect
  an AI assistant, *you* are directing your data to that service — Beeline is
  just the local conduit you control.

## Architecture

```
your MCP client ──(MCP, stdio)──┐
(AI assistant, you connect it)  ├──▶  beeline    ──▶ SQLite + Keychain (local)
CLI / desktop app ──────────────┘     core: accounts / auth / policy /
                                       storage / audit / local-ids
                                              │
                                       providers: Gmail · Microsoft Graph
```

## Workspace layout

| Crate              | Role                                                       |
|--------------------|------------------------------------------------------------|
| `crates/types`     | Normalized data model, camelCase JSON                      |
| `crates/providers` | `MailProvider` trait + Gmail and Microsoft Graph adapters  |
| `crates/storage`   | SQLite (accounts, local-ids, audit, confirmations) + Keychain |
| `crates/core`      | `MailAgent` facade, OAuth (PKCE), policy, local-id map      |
| `crates/mcp`       | MCP server: JSON-RPC 2.0 over stdio                        |
| `crates/control`   | Control API over a unix-domain socket (human-facing)       |
| `crates/cli`       | `beeline` binary                                         |
| `apps/desktop`     | Tauri app for account onboarding/management                |

## Quick start (CLI)

```sh
cargo build
cp config.example.toml ~/.beeline/config.toml   # add your OAuth client ids
./target/debug/beeline add-account --provider gmail
./target/debug/beeline add-account --provider microsoft
./target/debug/beeline search --account all "from:bruce"
./target/debug/beeline read <localMessageId>
```

OAuth client config is read from `~/.beeline/config.toml` (or env vars). See
`config.example.toml`.

## Connecting a client

Beeline exposes its tools over MCP on stdio. Point any MCP client at the binary:

```json
{
  "mcpServers": {
    "beeline": { "command": "/path/to/beeline", "args": ["mcp"] }
  }
}
```

For Claude Desktop, `beeline install-mcp claude` will write that entry for you
(it only edits the client's local config — it does not move any mail). Then
restart the client.

> Note: once you connect an AI assistant and ask it to read your mail, that
> message content is sent to **your AI provider** (e.g. Anthropic, OpenAI) so
> the assistant can reason over it. Beeline never sees or stores it — but it
> does leave your Mac at that point, by your choice. Beeline itself transmits
> nothing.

## Privacy

- No Beeline/AppCamp servers. Mail is fetched directly from your provider to
  your Mac and held only in memory to answer a request.
- OAuth tokens are stored only in the macOS Keychain, never on disk in plaintext
  and never transmitted off-device.
- Reads mail and creates drafts; never sends, deletes, or moves anything — you
  review and send drafts yourself.

Full [privacy policy](https://appcamp.com/privacy/) · technical
[data flow & security](./docs/security.md).
