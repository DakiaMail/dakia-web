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

## Work checklist

- [x] Read approved specification and repository instructions; move clean task branch to current canonical upstream.
- [x] Check current official authentication/transport documentation.
- [ ] Persistence, management API, OAuth and redacted audit.
- [ ] Concrete bounded tool schemas and mail operations.
- [ ] Durable send deduplication and ambiguity tests.
- [ ] Settings and client setup UI; keyboard and visual verification.
- [ ] Disposable synthetic IMAP/SMTP, built HTTP app and protocol verification.
- [ ] Local builds/tests, draft PR and exact-head CI; owner review.

This document is a progress record, not a claim that unchecked features work.
