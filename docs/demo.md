# Public demo

The public sandbox at https://demo.dakiamail.com runs the actual web app. Its
bootstrap installs a local Fetch/EventSource transport before mounting React.
Normal builds omit that bootstrap. There are no demo-specific screens or labels.

`src/demo` implements the existing API DTOs using a fictional mailbox in
IndexedDB. Per-origin Web Locks serialize requests across tabs. Local events
refresh the existing query graph; scheduled sends advance while a tab is open
and catch up on the next visit. No delivery worker, mail provider or real
account is connected. Unknown requests fail locally, never fall through to a
server. Attachments download through local Blob URLs. Notification permission
requests are rejected before calling browser permission APIs.

Visitor state persists between compatible app updates. The storage envelope is
versioned independently of the app release; unsupported or corrupt envelopes
reseed the mailbox. Storage denial falls back to memory without adding UI copy.
The website's fresh-start link opens `/?reset=1`, clears sandbox-origin storage
and caches, then removes the parameter before mounting the app.

## Development and checks

```sh
npm ci
npm test
npm run build
npm run build:demo
npm run test:demo
npm run test:parity
```

Browser tests use installed Google Chrome; set `CHROME_PATH` on macOS. The
parity check renders the normal production build against the same sample API
and compares screenshots with a 50-pixel allowance for browser rasterization noise. Acceptance tests exercise the sandbox
without request mocking and reject API/auth traffic. Captures cover both themes,
narrow screens, and short viewports. To check an existing deployment:

```sh
DEMO_URL=https://demo.dakiamail.com npm run test:demo
```

## Publishing

GitHub Actions `Demo` runs on every main update and pull request. It tests both
builds, then deploys the exact verified demo artifact from main to the separate
Cloudflare Pages project `dakia-demo`. A candidate deployment must also pass
Chrome acceptance before the live branch is updated. Its version endpoint records the source
commit. A failed check never replaces the live demo. No local deployments.

Required Actions secrets in `DakiaMail/dakia-web`:

- `CLOUDFLARE_ACCOUNT_ID`: the account containing `dakia-demo`.
- `CLOUDFLARE_API_TOKEN`: a dedicated token with Cloudflare Pages Edit on that account.

The Pages custom domain must be `demo.dakiamail.com`, with a CNAME targeting
`dakia-demo.pages.dev`. The project has no API proxy, provider credentials or
Pages Functions. Demo-only response headers restrict resources, disallow form
submissions and indexing, and prevent caching the version endpoint. A top-level
404 page disables the host’s SPA fallback, so unknown `/api/*` and `/auth/*`
paths return 404, including direct browser navigations.

Revert a bad frontend commit on main to publish a tested rollback. Keep the
previous Cloudflare deployment available for emergency rollback. Publish the
website's Try demo link only after the demo domain and acceptance tests pass.
