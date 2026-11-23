# Agent connections and remote MCP

## Connection permissions

Remote MCP is built into Dakia. Templates are excluded. A named connection belongs to one user and automatically sees all their current and future accounts and all drafts, including human-created drafts. It never sees another user's data.

Manual connections use a random token shown once. Dakia stores only its SHA-256 hash. OAuth connections use access and refresh tokens. Sending and deleting are independent controls, both off by default. Users can rename, edit permissions and revoke individual connections. Permission edits and revocation take effect on the next request, including cached clients. Restricted tools are hidden and checked independently on invocation.

Baseline access includes account/folder discovery, search, messages, threads, attachments, draft creation/editing, reply/forward preparation, archive, read/unread, stars and labels. Sending covers new messages, replies, forwards and existing drafts. Deletion covers Trash, permanent deletion, empty Trash, folders and drafts. No arbitrary REST proxy or generic mutation tool is exposed. Mixed-effect operations require every relevant permission.

Every send uses a durable request identifier. Identical retries must not resubmit; changed content with the same identifier conflicts. An uncertain SMTP result stays unknown and is never automatically retried. This is duplicate protection, not exactly-once SMTP delivery.

Backend audit records metadata and denied attempts, never bodies, attachments, credentials or headers. There is no activity UI.

## Architecture decisions

- Stateless Streamable HTTP at `/mcp`, JSON responses, bearer authentication on every request. No session authorization cache.
- Connection management is inside the existing cookie-session and CSRF-protected API.
- OAuth authorization-code flow with S256 PKCE, exact registered redirect URI and resource binding. Browser approval creates a connection or uses an existing connection owned by the signed-in user. Access/refresh grants remain bound to that connection and revocation.
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

Open Workspace → Agent connections, choose your client and copy the server URL. The browser approval creates a connection with sending and deletion disabled. To reuse an existing connection, select it on the approval page. Connections and their permissions can be changed or revoked in Workspace.

Use **Check server URL** to test HTTPS and OAuth discovery from your browser. To verify public access, run these from outside your network and VPN:

```sh
curl --fail https://YOUR_DAKIA_HOST/.well-known/oauth-protected-resource/mcp
curl -i https://YOUR_DAKIA_HOST/mcp \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"ping"}'
```

Expect resource metadata for your exact server URL and a `401 Unauthorized` response with a `WWW-Authenticate` header that points to that metadata.

### Claude

In Claude → Customize → Connectors, add a custom connector named **Dakia** and paste the server URL. If prompted, choose **Register automatically**. Choose **Connect** and approve access in Dakia. See [Claude’s custom connector guide](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp) for workspace-specific steps.

### Claude Code

```sh
claude mcp add --transport http dakia https://YOUR_DAKIA_HOST/mcp
```

Run `/mcp` in Claude Code and approve access in Dakia.

### ChatGPT

On [ChatGPT Plugins](https://chatgpt.com/plugins), select **Add custom MCP server**. Enter **Dakia** as the name, paste the server URL and choose OAuth with dynamic registration. Approve access in Dakia, install the plugin and select it with @ in a conversation.

Dakia advertises its name and PNG through MCP. ChatGPT also has its own name field when creating a custom server, and distributed plugins have their own icon settings. Use the [ChatGPT setup guide](https://developers.openai.com/plugins/deploy/connect-chatgpt) to refresh an existing custom server. For a packaged plugin, use the Dakia PNG as the logo and composer icon in the [plugin manifest](https://developers.openai.com/plugins/deploy/submission#icons).

### Hermes Agent

Copy the server URL from Dakia, then run:

```sh
hermes mcp add dakia --url "https://YOUR_DAKIA_HOST/mcp" --auth oauth
```

Approve access in your browser, choose the tools to enable, then start a new Hermes session. To sign in again later, run `hermes mcp login dakia`. For a remote or headless Hermes installation, see [OAuth over SSH](https://hermes-agent.nousresearch.com/docs/guides/oauth-over-ssh).

### Other AI agents

Use Streamable HTTP and OAuth with automatic registration. If your client requires a bearer token, choose **Other AI agent → Use a token instead** in Dakia and enter the generated `Authorization` header in the client.

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
