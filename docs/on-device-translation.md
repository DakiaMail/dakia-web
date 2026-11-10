# Translation location and saved language packs

Installation owners can choose where translations run in **Workspace → Translation**:

- **On devices:** translation runs in the reader's browser. If it cannot finish, the reader shows an error and lets the user retry. Host translation is blocked by the backend, including direct API requests and queued work.
- **On devices with host fallback:** try the browser first, then use the host if the browser cannot download or run the language pack, runs out of storage, or cannot finish. The reader explicitly reports when host translation is used. Cancellation and a busy translator do not trigger fallback.
- **On the host:** use the existing host translation worker. This remains the default for existing installations. Devices do not download translation weights in this mode.

The choice applies to everyone on this Dakia installation and survives server restarts. Ordinary users can see the policy but cannot edit it. Switching to device-only mode prevents new host jobs and cancels queued translation jobs; active host translation checks the policy every 500 ms and discards its subprocess/result when host translation is disabled. Classification is unaffected.

## Administration

The local installation owner has permission automatically. For an OIDC installation, the operator must explicitly nominate administrators using comma-separated stable Dakia user IDs:

```sh
DAKIA_ADMIN_USERS=user-id-one,user-id-two
```

Use the `user_id` from `/api/session`, not an email address or OIDC display name. The compose file forwards this setting to the container. This does not grant access to another person's mailboxes. No first-visitor or email-domain-based promotion is performed.

The authenticated `GET /api/translation/settings` response includes `mode` (`host`, `device`, or `automatic`) and `can_manage`. Only an administrator can `PUT` a new mode. Normal origin and CSRF checks apply. The policy is stored in the existing control database settings table; no data migration is required. Invalid/corrupt policy data fails closed rather than silently allowing host work.

## Reader experience

Translation still starts only when the reader chooses **Translate to English**. The existing source-language selector, original/translation toggle, safe mail renderer and image privacy controls remain in place.

The first local translation downloads the selected language's public weights and the shared WASM runtime. A circular indicator shows verified/completed bytes and incoming download progress, with a keyboard-reachable cancel action. Preparation and translation use an indeterminate indicator with reduced-motion support. Cancelling returns keyboard focus to the translation button.

Each artifact is checked against its pinned byte size and SHA-256 digest before it is saved or executed. Cached artifacts are rechecked before loading a fresh worker. Corrupt cached artifacts are removed and downloaded again. A canceled partial artifact is not installed; completed artifacts can be reused on retry.

Language packs live in browser Cache Storage (`dakia-translation-packs-v1`) and survive ordinary page reloads and browser restarts. Public weights remain saved after sign-out; message text and translated output do not enter this cache. The packs are independent of the bounded recent-mail IndexedDB cache. The browser may evict them under storage pressure, and clearing site data removes them. Private browsing may not preserve them when its session ends.

Workspace lists complete saved packs and lets any user remove weights from their own browser. Removal preserves artifacts shared with another complete saved language. A shared runtime can remain cached for other languages.

One browser worker is active at a time. It uses Firefox's optimized runtime, transferable model buffers, input-buffer detachment and disabled sentence caching. Downloads fill one bounded buffer directly instead of retaining and combining network chunks. It releases native response handles after each translation. The reader cancels work and releases the worker when its content/account changes, it unmounts, the user cancels, or the session expires. A warm idle worker is released after fifteen seconds. Downloaded files remain saved, so reuse after idle does not need another network download.

The implementation uses actual worker availability and runtime errors, not an unverified minimum-RAM cutoff. A bounded worker deadline makes automatic fallback possible when the browser stalls. An operating-system termination of the entire browser tab cannot be recovered inside that tab. Other browsers and lower-memory devices still need qualification; the earlier physical Android study is evidence for the runtime, not universal device certification.

## Host and asset delivery

Authenticated browsers obtain only hash-allowlisted public weights from `/api/translation/models/{sha256}`. The endpoint does not accept mail text or arbitrary URLs. The host fetches pinned Mozilla artifacts with size/hash verification, bounded response sizes, no redirects and a download deadline, then caches valid public files in `DAKIA_MODEL_CACHE`. The default outside the container is a temporary model cache; production containers use `/data/models` as before.

The WASM is bundled as a versioned static asset and verified by the browser. It is excluded from automatic service-worker app-shell prefetch, so installing or visiting Dakia does not download the 5 MB engine or language packs automatically. The module worker's small JavaScript bundle is an ordinary static app asset. Model/runtime cache writes are performed explicitly by the translation client; the service worker still does not cache mail or private API responses.

Strict deployment CSPs must permit same-origin workers and WebAssembly compilation (`script-src 'self' 'wasm-unsafe-eval'`). No general `unsafe-eval`, remote script origin or cross-origin isolation is required. Keep existing stronger directives for rendering mail. Prototype and production runtime provenance, the two loader compatibility changes and the full MPL license are in `workers/translation/browser/`.

## Verification

The focused frontend tests cover policy-gated local inference, progress semantics, cancellation, original-message toggling, late-result suppression, and explicit automatic fallback. Server tests cover administrator permissions, CSRF, durable policy, invalid settings and hash allowlisting. Dedicated Chrome/backend scenarios exercise the real browser engine, saved-pack reuse after reload with the browser disconnected, corrupt-pack recovery, backend enforcement, settings persistence, real host fallback and keyboard operation. The offline case loads its authenticated message before disconnecting; it does not claim offline sign-in or initial session startup.

Run from the repository root:

```sh
npm ci --ignore-scripts
npm ci --prefix workers/translation --ignore-scripts
npm ci --prefix workers/push --ignore-scripts
npm test
npm run build
cargo test -p dakia-server --lib --locked
```

Then from `apps/web`, with the installed Google Chrome path:

```sh
CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' npx playwright test --config playwright.device-translation.config.ts
```

This starts a real loopback Rust fixture with synthetic mail on port 4346. It never connects a real mailbox. Public language weights may be downloaded into `/private/tmp/dakia-production-translation-models` when missing; set `DAKIA_MODEL_CACHE` to another disposable directory if needed.
