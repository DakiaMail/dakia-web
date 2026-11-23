# MCP verification

This page records the original MCP fixtures and their coverage during development of [PR #58](https://github.com/DakiaMail/dakia-web/pull/58).

## Actual execution

- Full `cargo test --workspace`: exit 0, including the MCP API, executor, storage and provider suites. Latest full local log: `target/mcp-final-workspace.log`. Optional model/PostgreSQL fixtures retain their existing explicit skips.
- `NODE_OPTIONS=--no-experimental-webstorage npm test`: all 335 tests passed. The option disables Node 26's experimental global storage, which interferes with the existing jsdom suite. CI uses Node 24.
- `npm run build`: TypeScript and production Vite build passed. Existing large-chunk warning remains.
- Four focused MCP API tests pass, including connection isolation, token-once, independent permissions, all drafts/new accounts, concurrent/reopened send IDs, and OAuth PKCE/code reuse/resource/client binding/refresh rotation/revocation.
- Queued mutation regression passes for revoked connections, disabled deleting and archive mappings resolving to Trash.
- Three TLS IMAP wire tests pass: UID-scoped permanent deletion/readback, refusal without UIDPLUS, and refusal to claim Gmail permanent deletion outside its advertised Trash.
- Local synthetic Graph HTTP fixture verifies the permanentDelete endpoint and absence readback; a still-present message returns unknown. This is not a live Microsoft service test.

## Built-app disposable instance

`target/mcp-disposable-69fb069d54/results.json` records 16 passed checks against the compiled server over HTTP, pinned disposable Dovecot, and a loopback TLS SMTP sink:

1. Default-off private connection creation.
2. All four permission combinations, hidden tools and cached direct-call denial.
3. Human drafts, cross-user isolation, generic/batch rejection.
4. Real TLS IMAP folders, search, messages and attachments.
5. Thread read, reply/forward drafts with attachments, draft editing and baseline read/star/category/archive without send/delete permission.
6. Extra-field and mixed-effect bypass rejection.
7. Newly added fixture account visible only to its owner.
8. Reply/forward sending uses the same send gate.
9. Trash, explicit bounded empty Trash and draft deletion work independently of sending.
10. Four concurrent duplicate sends produce one SMTP acceptance; restart/retry does not resend.
11. Lost SMTP acknowledgement persists delivery_unknown; restart/retry does not cause another SMTP attempt.
12. Preaccept recipient refusal persists failed, not unknown.
13. Folder deletion is read back.
14. UID-scoped permanent deletion is read back.
15. Audit excludes fixture body sentinels, credentials and bearer token.
16. Revocation immediately rejects the cached client and records redacted authentication denial.

The sink observed three envelopes: one confirmed acceptance, one consumed body with deliberately lost acknowledgement, and one refusal before DATA. Two bodies were consumed, not two confirmed deliveries. This is not exactly-once SMTP delivery.

## Real Chromium and official SDK

`browser/results.json` records three passing keyboard journeys: create and all client instructions; independent permission edits/rename/focus/persistence; token not redisplayed, cancel/Escape and revoke; OAuth consent initial focus and real browser form approval/cancellation. The registered `.invalid` callback intentionally has no DNS. Browser tests assert the real 303 response and Location, not a working external client callback. API and SDK tests complete code exchange separately.

Ten genuine screenshots cover create, edit, revoke, client setup, settings and OAuth in light/dark and 390×540 narrow/short viewports. Captures wait for finite animations to settle. The browser checks exposed and fixed both Origin:null from no-referrer form policy and CSP blocking registered OAuth redirects. Modal replacement now preserves the original trigger through setup and closing.

Official TypeScript MCP SDK **1.32.2** passed four checks in `sdk-results.json`: Streamable HTTP bearer initialization/tools/calls; OAuth discovery/dynamic registration/resource-bound PKCE; refresh rotation; current permission edits, direct cached-tool denial and revocation.

No actual authenticated ChatGPT or Claude Code session was performed. Private login is owner-only. Public HTTPS reachability, account/workspace eligibility and vendor UI policies still need the owner's client setup check. No live Gmail or Microsoft mailbox was used.

## Reproduce

```sh
npm ci --ignore-scripts
npm ci --prefix workers/push --ignore-scripts
npm ci --prefix workers/translation --ignore-scripts
npm run build
cargo build -p dakia-server --bin dakia-server --example mcp_fixture
python3 scripts/mcp-disposable.py --serve
# Use the new fixture directory printed by that command:
node scripts/mcp-browser.mjs target/mcp-disposable-<id>/browser-state.json
npm install --prefix target/mcp-client --no-save @modelcontextprotocol/sdk@1.32.2
NODE_EXTRA_CA_CERTS="$PWD/target/mcp-disposable-<id>/cert.pem" \
  node scripts/mcp-sdk.mjs target/mcp-disposable-<id>/browser-state.json
```

Requires Docker, OpenSSL, Chromium (`CHROME_PATH` may override `/usr/bin/chromium`) and Node >=22.22.2 in PATH. The fixture uses synthetic users/cookies only, refuses an existing data directory, binds loopback listeners, and cleans its server/container/listeners on termination while retaining evidence. `--serve` keeps it available for browser checks until stopped. No production or personal instance was accessed.

## CI

All five required execution checks passed for checkpoint `38f4d3e`: upgrade checks, classification checks, Demo verify, amd64 repair image and arm64 repair image. Production image/manifest/deploy jobs were skipped by PR conditions. Final exact-head CI must be checked on the PR after the last commit; this document does not substitute older-head results for it.
