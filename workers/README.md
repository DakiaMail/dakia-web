# Translation and push workers

These are separate, sequential newline-delimited JSON subprocesses supervised by the Rust server. Translation and push must not share a process. Both accept job/scope identities established by the authorized server and return bounded results or generic errors without logging mail contents or secrets.

## Install and test

```sh
npm ci --prefix workers/translation --ignore-scripts
npm ci --prefix workers/push --ignore-scripts
node --test workers/translation/test/*.test.mjs workers/push/test/*.test.mjs
```

Real translation-model tests are opt-in and download public pinned artifacts when absent:

```sh
DAKIA_REAL_MODELS=1 DAKIA_MODEL_CACHE=/operator-owned/model-cache node --test workers/translation/test/models.test.mjs
```

Set `DAKIA_WORKERS` to this directory and optionally `DAKIA_NODE` for server operation. For direct protocol debugging with synthetic input only:

```sh
DAKIA_MODEL_CACHE=/operator-owned/model-cache node workers/translation/main.mjs
DAKIA_VAPID_FILE=/operator-owned/vapid.json node workers/push/main.mjs
```

The private VAPID file contains `subject`, `publicKey` and `privateKey`. The provisioning helper requires `DAKIA_VAPID_FILE` and `DAKIA_VAPID_SUBJECT` (an operator contact HTTPS URL or mailto URI) and uses create-only writes, preserving existing keys. Never commit keys or pass their contents in shell arguments. The Rust server owns persistent subscriptions, deduplication, retry leases and account/user authorization.

## Translation

`models.json` pins public-model URLs, sizes, paths and SHA-256 hashes. Only verified public weights are shared across accounts. Translation resets the WASM instance per request, disables sentence caching and sanitizes HTML before and after processing. Requests are bounded by size and deadline. Missing/download-failed/corrupt artifacts fail visibly instead of returning invented translations. `vendor/` contains the Bergamot runtime and its upstream package metadata; preserve its MPL notices.

## Push

The service sends generic New mail notifications, not sender/subject/body data. The supported endpoint allowlist and HTTPS validation live in `push/service.mjs`; subscription ownership is checked server-side. Tests use stubbed transport rather than sending external notifications. A passing worker test is not proof of operating-system banner display or live browser permission.

See the root third-party notices and preserved licenses for model/runtime provenance.
