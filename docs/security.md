# Beeline — data flow & security

The engineering companion to the [privacy policy](https://appcamp.com/privacy/).
It describes, concretely, what data Beeline touches and where it goes.

## Principles

- **No servers.** AppCamp operates no backend for Beeline. Nothing is sent to,
  stored by, or processed by AppCamp.
- **Client-only handling of provider data.** Mail and OAuth tokens are handled
  only on the user's device. This keeps Beeline on the exempt side of Google's
  restricted-scope security-assessment trigger (which applies to apps that store
  or transmit restricted-scope data on their own servers).
- **Read + draft only.** Beeline reads mail and creates drafts; it never sends,
  deletes, archives, or moves anything. Those actions are not exposed — the user
  reviews and sends drafts themselves from their mail app.
- **Transmits nothing off-device.** The only way mail leaves the Mac is through a
  client the *user* connects (see "User-directed transfer").

## Data flow

```
your MCP client ──(MCP, stdio)──┐
(AI assistant, user-connected)  ├──▶  beeline    ──▶  Gmail API / Microsoft Graph
CLI / desktop app ──────────────┘      │              (direct, over TLS)
                                        ▼
                              Keychain (tokens) + SQLite (metadata), local only
```

Provider data is fetched directly from Google/Microsoft to the device. Beeline is
never an intermediary server.

## Where data lives

| Data | Location | Notes |
|------|----------|-------|
| OAuth refresh/access tokens | macOS Keychain (`com.appcamp.beelinemailagent`) | Never on disk in plaintext; never transmitted off-device |
| Account records, local-id map, audit log, pending confirmations | SQLite at `~/.beeline/beeline.sqlite` | No message bodies; no tokens |
| Message bodies | In memory, transiently | Fetched on demand to answer a request; not cached to disk by default |

## OAuth scopes

| Provider | Scopes | Flow |
|----------|--------|------|
| Google (Gmail) | `gmail.readonly` + `gmail.drafts.create` (+ offline access) | Auth-code + PKCE, loopback redirect; desktop client |
| Microsoft (Graph) | `openid profile offline_access Mail.ReadWrite` | Auth-code + PKCE, loopback redirect; public client, `common` authority |

`gmail.drafts.create` is create-only and **cannot send** (narrower than
`gmail.compose`); `Mail.ReadWrite` is the minimal Graph scope that allows draft
creation. **No send tool is exposed** — Beeline creates drafts and never sends.
Send / archive / move, if added later, are gated
by the confirmation flow.

## User-directed transfer

Beeline exposes a **local** MCP server (`beeline mcp`) over stdio. If the user
points an AI assistant at it, then when that assistant reads a message, the content
is sent to the assistant's provider (e.g. Anthropic, OpenAI) so it can act on the
request. This transfer is initiated and controlled by the user, governed by that
provider's policy; Beeline neither brokers nor observes it. Beeline only serves the
local interface the user connected.

## Logging

The local audit log records *mutation events* (account added/removed, permission
changes, and future send/archive/download actions) — never message bodies, never
tokens. Diagnostic output redacts tokens and contents.

## Revocation & deletion

- Disconnecting an account in Beeline deletes its Keychain token and local records.
- Removing `~/.beeline` deletes all locally stored data.
- Access can be revoked independently from the user's Google or Microsoft account.
