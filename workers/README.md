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

The private VAPID file contains `subject`, `publicKey` and `privateKey`. Native runtime and container entrypoints enable server-side Web Push automatically before starting the server. They use `vapid.json` beside `DAKIA_CREDENTIAL_KEY_FILE` (normally `/var/lib/dakia-secrets/vapid.json` natively or `/secrets/vapid.json` in Compose), with `DAKIA_ORIGIN` as the HTTPS contact subject. Override `DAKIA_VAPID_FILE` or `DAKIA_VAPID_SUBJECT` for an existing operator-owned key file or another HTTPS/mailto contact. Fresh provisioning needs a writable persistent secrets directory owned by the server user. No shared key is shipped.

The helper validates existing private regular files without modifying them. New keys are mode 0600, created as the service user and atomically published with a create-only hard link, so concurrent provisioning never replaces a key or reads a partial write. Keep the file on a local filesystem supporting hard links. Existing valid keys need no subject override and can be mounted read-only. Invalid, unreadable, symlinked or group/other-readable key files, invalid/missing first-start subject, missing dependencies and write failures stop startup rather than silently disabling notifications. Repair permissions or configuration without deleting existing keys. Preserve keys and their configured paths across upgrades and backups to retain subscriptions.

For direct helper use, set `DAKIA_VAPID_FILE` and, for a new file, `DAKIA_VAPID_SUBJECT`. Direct invocation of `dakia-server` bypasses the supported launchers and still requires explicit Web Push configuration. Never commit keys or pass their contents in shell arguments. The Rust server owns persistent subscriptions, deduplication, retry leases and account/user authorization.

Server readiness does not grant browser permission or subscribe devices. Users must choose Enable notifications on each device over HTTPS; iOS/iPadOS requires a supported installed Home Screen web app. No test notification is sent by provisioning.

## Translation

`models.json` pins public-model URLs, sizes, paths and SHA-256 hashes. Only verified public weights are shared across accounts. Translation resets the WASM instance per request, disables sentence caching and sanitizes HTML before and after processing. Requests are bounded by size and deadline. Missing/download-failed/corrupt artifacts fail visibly instead of returning invented translations. `vendor/` contains the Bergamot runtime and its upstream package metadata; preserve its MPL notices.

## Push

The Web Push payload is generic and omits sender, subject, and body text. A signed-in browser can separately fetch an authorized message summary and use it to display sender, subject, account, and preview text in the notification. Without that summary it displays generic New mail text. The supported endpoint allowlist and HTTPS validation live in `push/service.mjs`; subscription ownership is checked server-side. Tests use stubbed transport rather than sending external notifications. A passing worker test is not proof of operating-system banner display or live browser permission.

See the root third-party notices and preserved licenses for model/runtime provenance.
