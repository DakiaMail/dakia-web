# MCP implementation verification checkpoint

Task `t_a24029b1`, draft PR <https://github.com/DakiaMail/dakia-web/pull/58>.
Not yet ready for owner review, merge or deployment.

## Executed successfully

- `cargo test -p dakia-server --test api mcp:: -- --nocapture`: four tests passed, covering private connection lifecycle, default-off permissions, four independent permission combinations, cached direct-call denial/revocation, all-human-draft access, new accounts, user isolation, concurrent/reopened send ID bindings, cross-account ID conflicts, draft deletion and browser OAuth PKCE/single-use codes/revocation.
- `cargo test -p dakia-server --lib executor::tests -- --nocapture`: three tests passed, including SMTP-boundary permission enforcement and durable submission outcomes.
- `cargo test -p dakia-mail-provider --test mcp_delete_fixture -- --nocapture`: two actual TLS IMAP wire tests passed. UID-scoped deletion verifies absence; servers without UIDPLUS are rejected rather than using global EXPUNGE.
- `npm test --workspace @dakia/web -- --run src/AgentConnections.test.tsx`: create, token-once, edit and revoke component test passed.
- `npm run build`: TypeScript and production Vite build passed. Existing large-chunk warning remains.
- `cargo build -p dakia-server --bin dakia-server --example mcp_fixture`: actual server and explicit new-directory fixture seeder built successfully.
- `python3 scripts/mcp-disposable.py`: eleven checks passed against the built server over HTTP, pinned disposable Dovecot and a loopback TLS SMTP sink. Server restarted twice during the test. Docker container, server and listeners were removed in finally cleanup.

## Disposable run: actual sanitized results

Evidence retained locally in `target/mcp-disposable-453a22e4f8/results.json` and `server.log`:

1. Built HTTP app creates default-off private connection.
2. Four independent permission combinations, hidden tools and cached direct-call denial.
3. All human drafts, cross-user isolation and generic/batch bypass rejection.
4. Real TLS IMAP folder discovery, search, message and attachment retrieval.
5. Four concurrent duplicates share one submission and produce one SMTP acceptance. Retrying after server restart does not send again.
6. Lost SMTP acknowledgement persists `delivery_unknown`; retry/restart produces no additional SMTP attempt.
7. Preaccept SMTP recipient refusal persists `failed`, not unknown.
8. Disposable IMAP folder deletion is read back.
9. UID-scoped permanent deletion is read back.
10. Audit rows contain no fixture body sentinel, credentials or bearer token.
11. Revocation invalidates the cached HTTP client immediately.

SMTP sink observed three envelopes: one confirmed acceptance, one consumed body with deliberately lost acknowledgement, and one refusal before DATA. Thus two bodies were consumed, not two confirmed deliveries. No claim of exactly-once SMTP delivery.

Only synthetic fixture users, accounts, messages and credentials were used. No owner mail, personal instance, production system, authenticated ChatGPT session or Claude Code session was used.

## Still required

- Complete security edge-case review and regressions, notably archive mappings resolving to Trash, permission changes while queued Trash is pending, accurate audit outcomes for nested partial/unknown deletion results and denied/revoked authentication, and OAuth refresh/resource/redirect rejection coverage.
- Review Gmail permanent deletion semantics and Graph deletion tests. The current isolated live deletion smoke covered generic IMAP, not Gmail or Microsoft services.
- Exercise remaining mail tools, reply/forward drafts, archive/read/star/category, Trash/empty Trash, mixed-effect and extra-field bypass cases over live HTTP.
- Browser create/edit/revoke keyboard-only paths, focus, light/dark/narrow/short viewport screenshots, personal screenshot inspection and OAuth consent-page visual review. Component tests are not visual evidence.
- Official MCP SDK/client harness and honest client compatibility limits. No private client login should be attempted by the worker.
- Full workspace/frontend regression results, current PR exact-head CI, final PR body and owner review handoff.

Use `python3 scripts/mcp-disposable.py --serve` to retain the task-owned browser fixture after its smoke run. It prints its strict loopback HTTPS URL; `browser-state.json` contains the test-only session cookie. Stop that harness to clean its resources. Each run requires a new fixture directory; the seeder refuses existing directories.
