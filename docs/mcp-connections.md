# Connect an AI assistant to Dakia

An agent connection lets an assistant search your mail, read threads and attachments, and prepare drafts in Dakia. It uses Model Context Protocol (MCP), a standard for connecting assistants to apps.

## Get your server URL

1. In Dakia, open **Workspace → Agent connections**.
2. Choose your assistant and select **Copy URL**. The address ends in `/mcp`.
3. Follow the instructions for your assistant below. When Dakia opens in your browser, sign in and choose **Connect**.

For a cloud assistant, use an HTTPS address that the assistant can reach from its own network. For Hermes or Claude Code running on your computer, the server must be reachable from that computer.

## Client setup

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

## Manage access

A new connection can read mail and attachments, prepare or edit drafts, and organize mail. This includes mail and drafts in accounts you add later under your Dakia login.

In **Workspace → Agent connections**, choose **Edit** next to a connection to rename it or change either permission:

- **Allow sending** lets the assistant send new messages, replies, forwards, and existing drafts.
- **Allow deleting** lets it move messages to Trash, permanently delete mail, empty selected messages from Trash, and delete folders or drafts.

Both permissions start off. To disconnect an assistant, choose **Revoke** next to its connection. Revoking a connection stops further requests from apps that use it. To use an existing connection when signing in again, select it on the approval page.

## Troubleshoot a connection

| Problem | What to check |
| --- | --- |
| The assistant can’t reach Dakia | Use **Check server URL** in Workspace. It checks from your browser. Test from outside your network or VPN if the assistant runs in the cloud. |
| Setup asks for authentication | Choose OAuth with automatic registration where the client offers it. For a manual-token client, use **Other AI agent → Use a token instead**. Dakia shows that token once. |
| Send or delete tools are missing | Check the connection’s permissions in Workspace, then refresh the tools in your assistant. |

To confirm HTTPS and OAuth discovery from another network:

```sh
curl --fail https://YOUR_DAKIA_HOST/.well-known/oauth-protected-resource/mcp
curl -i https://YOUR_DAKIA_HOST/mcp \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"ping"}'
```

The first command returns metadata for your server URL. The second should return `401 Unauthorized` with a `WWW-Authenticate` header pointing to that metadata.

## How the connection works

- Stateless Streamable HTTP at `/mcp`, JSON responses, bearer authentication on every request. No session authorization cache.
- Connection management is inside the existing cookie-session and CSRF-protected API.
- OAuth authorization-code flow with S256 PKCE, exact registered redirect URI and resource binding. Browser approval creates a connection or uses an existing connection owned by the signed-in user. Access/refresh grants remain bound to that connection and revocation.
- Support public-client dynamic registration for existing clients. Do not fetch arbitrary client metadata URLs, avoiding an SSRF surface. Advertise only implemented registration mechanisms.
- All persistence belongs behind the typed control repository with native SQLite and PostgreSQL statements and migrations.
- Reuse authorized-account capabilities, mail runtime, durable drafts and submission state machines. Never fabricate a browser session or forward arbitrary API paths.
- Audit metadata retained for 90 days; connection/send deduplication tombstones retained until user removal. Request identifiers are scoped to a connection and never recycled automatically.

Dakia stores only the hash of a manual token. OAuth connections use access and refresh tokens.

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
