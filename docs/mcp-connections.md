# Agent connections and remote MCP

Implementation checkpoint, task `t_a24029b1`. No production deployment or merge is authorized.

## Agreed product contract

Remote MCP is built into Dakia. Templates are excluded. A named connection belongs to one user and automatically sees all their current and future accounts and all drafts, including human-created drafts. It never sees another user's data.

Each connection has a random credential shown once. Store only its SHA-256 hash. Sending and deleting are independent controls, both off by default. Users can rename, edit permissions and revoke individual connections. Permission edits and revocation take effect on the next request, including cached clients. Restricted tools are hidden and checked independently on invocation.

Baseline access includes account/folder discovery, search, messages, threads, attachments, draft creation/editing, reply/forward preparation, archive, read/unread, stars and labels. Sending covers new messages, replies, forwards and existing drafts. Deletion covers Trash, permanent deletion, empty Trash, folders and drafts. No arbitrary REST proxy or generic mutation tool is exposed. Mixed-effect operations require every relevant permission.

Every send uses a durable request identifier. Identical retries must not resubmit; changed content with the same identifier conflicts. An uncertain SMTP result stays unknown and is never automatically retried. This is duplicate protection, not exactly-once SMTP delivery.

Backend audit records metadata and denied attempts, never bodies, attachments, credentials or headers. There is no activity UI.

## Architecture decisions

- Stateless Streamable HTTP at `/mcp`, JSON responses, bearer authentication on every request. No session authorization cache.
- Connection management is inside the existing cookie-session and CSRF-protected API.
- OAuth authorization-code flow with S256 PKCE, exact registered redirect URI and resource binding. Browser approval selects an existing user-owned connection; OAuth credentials cannot increase its permissions. Access/refresh grants remain bound to that connection and revocation.
- Support public-client dynamic registration for existing clients. Do not fetch arbitrary client metadata URLs, avoiding an SSRF surface. Advertise only implemented registration mechanisms.
- All persistence belongs behind the typed control repository with native SQLite and PostgreSQL statements and migrations.
- Reuse authorized-account capabilities, mail runtime, durable drafts and submission state machines. Never fabricate a browser session or forward arbitrary API paths.
- Audit metadata retained for 90 days; connection/send deduplication tombstones retained until user removal. Request identifiers are scoped to a connection and never recycled automatically.

## Official client evidence

Checked 20 November 2026:

- [MCP authorization](https://modelcontextprotocol.io/specification/latest/basic/authorization): OAuth 2.1, protected-resource metadata, authorization-server discovery, resource indicators and PKCE. Dynamic registration remains supported for backward compatibility.
- [Claude Code MCP](https://code.claude.com/docs/en/mcp): remote HTTP recommended; `claude mcp add --transport http NAME URL`; bearer headers and browser OAuth supported.
- [ChatGPT custom MCP servers](https://developers.openai.com/api/docs/guides/custom-mcp-server): Streamable HTTP and SSE supported; OAuth, unauthenticated or mixed authentication. No general bearer-token-entry setup is documented. OAuth is required for private Dakia access. Current UI uses Plugins → Add custom MCP server; workspace restrictions apply.

## Client setup

In Workspace → Agent connections, create a named connection and save the token shown once. Both permissions start off. Edit either independently or revoke only this connection. Access includes current/future accounts and all drafts.

### Claude Code

```sh
claude mcp add --transport http dakia https://YOUR_DAKIA_HOST/mcp
```

Run `/mcp` in Claude Code, authorize in your browser, sign in to Dakia and select the connection. Alternatively add `--header "Authorization: Bearer YOUR_SAVED_TOKEN"` using your saved token, treating the CLI configuration/history as sensitive. Never paste a real token into an issue or PR.

### ChatGPT

On ChatGPT web, open Plugins → Add custom MCP server. Enter `https://YOUR_DAKIA_HOST/mcp`, choose OAuth with dynamic registration, sign in to Dakia, select the named connection and Connect. Create/install the plugin and select it with @ in a conversation. The server must be reachable by ChatGPT over public HTTPS; localhost is only a disposable test endpoint. Workspace eligibility and controls depend on the ChatGPT account. Token entry is not promised. The owner must complete any private login; SDK verification is not a live ChatGPT session.

### Generic clients

Use Streamable HTTP POST at `/mcp`, with browser OAuth or `Authorization: Bearer YOUR_SAVED_TOKEN`. Authorization discovery is advertised by the 401 `WWW-Authenticate` challenge. Public-client registration, S256 PKCE, exact redirect validation, the `/mcp` resource indicator and rotating refresh credentials are implemented. No SSE-only legacy transport, arbitrary metadata URL fetching or unauthenticated mail access is provided.

## Tool contracts and limits

`tools/list` is authoritative for each connection. Every tool has a closed object schema, including nested draft objects; unknown fields/actions, arbitrary REST proxy calls and JSON-RPC batches are rejected.

- Discovery: `list_accounts` (offset/limit, max 100), `list_folders` (account_id).
- Reading: `search_messages` (account/folder/query/opaque continuation), `read_message`, `read_thread` (paged metadata), `get_attachment` (account-scoped identifier, base64 chunks up to 256 KiB).
- Drafts: `list_drafts` (max 100), `read_draft`, `save_draft` (exact revision, new UUID/revision 0), `prepare_reply`, `prepare_forward`. Forwarded attachments are pinned through the existing authorized received-part path.
- Organisation: `update_message` (exact revision and request_id; archive/read/unread/star/unstar/label), `get_operation` (poll queued outcome). Effective archive-to-Trash mappings require deletion permission. Pending operations recheck current connection authority before provider execution.
- Sending: `send_draft` sends a frozen draft revision, covering new/reply/forward/human drafts. `get_send_status` remains readable when sending is disabled. Request IDs are connection-wide permanent tombstones; identical retries share one durable submission; changed arguments conflict. Observe queued/accepted/failed/delivery_unknown, never generate an automatic replacement for unknown.
- Deleting: `trash_message`, `delete_message`, `empty_trash` (explicit maximum 50 known Trash message IDs, never expands its target set), `delete_folder` (special folders rejected), `delete_draft` (exact revision). Permanent destructive request claims persist across interrupted calls and cannot automatically replay uncertain writes. Generic IMAP requires UIDPLUS. Gmail messages must first be in the provider-advertised Trash for permanent deletion; expunge from other Gmail labels can archive instead. Provider failures/unknown and per-target results are returned honestly.

Two flags are independent and default off. Hidden tools are still denied directly. All requests reauthenticate without session authority caching; owner authorization scopes every account lookup. Backend audit records bounded action/outcome categories and hashed resource/request identifiers, not arbitrary attacker-selected names, message content, attachments, tokens or headers. Queued results are recorded as queued, not delivered; later status reads expose durable actual outcomes. Audit retention is 90 days and has no UI.

See [actual verification and compatibility limits](mcp-verification.md).

## Historical implementation checklist

- [x] Read approved specification and repository instructions; move clean task branch to current canonical upstream.
- [x] Check current official authentication/transport documentation.
- [ ] Persistence, management API, OAuth and redacted audit.
- [ ] Concrete bounded tool schemas and mail operations.
- [ ] Durable send deduplication and ambiguity tests.
- [ ] Settings and client setup UI; keyboard and visual verification.
- [ ] Disposable synthetic IMAP/SMTP, built HTTP app and protocol verification.
- [ ] Local builds/tests, draft PR and exact-head CI; owner review.

This document is a progress record, not a claim that unchecked features work.
