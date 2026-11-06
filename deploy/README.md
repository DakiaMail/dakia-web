# Deployment and operations

For new installations, start with [hosting](../docs/hosting.md) and the root Compose package. The root Dockerfile targets Linux AMD64 and ARM64. The legacy bundle packaging below targets Linux x86_64/glibc 2.39 (Ubuntu 24.04). It produces a local runtime image, not a hosted service or an automatic deployment workflow. Use an HTTPS reverse proxy. Existing installations can continue using an operator-controlled OIDC provider; new installations can use built-in local login. Keep the application private until access, provider behavior and backups have been validated for your environment.

## Configuration

Required environment variables (OIDC variables are required only when using OIDC):

| Variable | Meaning |
| --- | --- |
| `DAKIA_ORIGIN` | External HTTPS application origin |
| `DAKIA_OIDC_ISSUER` | Exact OIDC issuer URL, reachable from the server |
| `DAKIA_OIDC_CLIENT_ID` | Confidential client ID; register `<origin>/auth/callback` with PKCE |
| `DAKIA_OIDC_SECRET_FILE` | Path to the private client secret file |
| `DAKIA_CREDENTIAL_KEY_FILE` | Path to exactly 32 raw key bytes, separate from SQLite |
| `DAKIA_DATA_DIR` | Persistent local filesystem directory for control/account databases and blobs |

Optional/runtime settings:

| Variable | Default or purpose |
| --- | --- |
| `DAKIA_LISTEN` | `127.0.0.1:3025`; expose only through the intended proxy/network boundary |
| `DAKIA_WEB_ROOT` | `apps/web/dist`; set `/app/web` in the packaged image |
| `DAKIA_DURABLE_ATTACHMENT_BUDGET_BYTES` | 1073741824 per account; positive integer |
| `DAKIA_CLASSIFIER_DIR` | Model directory containing `MANIFEST.json` and verified artifacts |
| `DAKIA_WORKERS` | Node worker source/dependency directory |
| `DAKIA_NODE` | Node executable, default `node` |
| `DAKIA_MODEL_CACHE` | Writable cache for public translation models |
| `DAKIA_VAPID_FILE` | Private VAPID JSON file for Web Push; see `workers/README.md` |

Preserve key bytes across upgrades. Restrict secret directory/file permissions and mount secrets read-only. Never put credentials in command arguments, URLs, Git or logs. Give the service UID access to its data/model cache, not other application stores. Keep live SQLite on a local filesystem, not a network share. Run only one server/operator against a data directory; an exclusive lock enforces this.

## Build

Install the requirements and run the checks in the root README. Retrieve the oversized model first:

```sh
python3 scripts/fetch-classifier.py
sh deploy/build-bundle.sh
```

The builder installs locked npm dependencies, builds/tests the UI, tests the workers and Rust workspace, builds release binaries, and creates `deploy/bundle/runtime.tar.gz`. It packages copied binaries, web assets, workers and classifier artifacts, excluding account data and operator secrets. Build on Linux x86_64 using Node 24 LTS and the pinned Rust toolchain. The model fetch is explicit so offline builds can use a previously verified local artifact.

Extract the archive into a new build directory and build the included Dockerfile with an explicit local tag. It expects `bin/`, `web/`, `workers/` and `classifier/` beside it. The image runs as UID/GID 1000. Mount persistent storage at `/data`, set the required environment and mount private configuration files at their configured paths. Configure `DAKIA_WEB_ROOT=/app/web`; the image already sets the classifier, worker and model-cache paths. A private VAPID file is needed for push, not embedded in the image.

Use a read-only root filesystem, no new privileges, dropped capabilities and suitable resource limits. Networking must permit OIDC discovery/backchannel, mail provider TLS, selected model downloads and supported push endpoints. Do not bypass TLS verification to resolve routing failures. Ensure `/sw.js` serves JavaScript rather than the SPA fallback.

## Upgrade and verification

Keep the previous image and a matching backup before replacing the application. Retain the existing data and secret mounts, identity-provider state and proxy configuration. Never apply an old full proxy/compose file over unrelated operator changes.

Preserve incident stdout privately before replacement. Mail diagnostics are also retained in a bounded private ring under `DAKIA_DATA_DIR/diagnostics`, on the existing persistent mount. Keep Docker stdout rotation separately bounded. See the private diagnostics directory for correlation, uncertainty semantics, limits and privacy rules.

Worker updates must include their locked production dependencies. A healthy Rust
server does not establish that a Node subprocess can start. Run
`node /app/workers/translation/smoke.mjs` inside the final image with networking
disabled and no application data mounts before activation. This loads the HTML
parser, sanitizer and WASM engine. The supplied Dockerfile runs it as the service
user during the build. For translation acceptance, also run the smoke with
`--models` and `DAKIA_MODEL_CACHE` pointing at an isolated copy of verified public
Estonian models. This exercises real HTML translation without mail or credentials.

Verify the exact running image and application assets, health (`GET /healthz` identifies `dakia-web`), anonymous API rejection, owner login and safe read-only navigation. A healthy container alone does not prove provider, submission or backup acceptance. Test mail mutations only with deliberately disposable accounts/messages. Reconcile uncertain send outcomes before retrying. Sent filing failures must retry filing only, not SMTP delivery.

The local healthcheck binary can be used as:

```sh
/app/dakia-healthcheck
```

## Backup and restore

Stop the server before invoking the offline operator with its matching binary and key. Do not run it against a concurrently active store.

```sh
dakia-admin backup DATA_DIR NEW_BACKUP_DIR KEY_FILE
dakia-admin restore BACKUP_DIR NEW_DATA_DIR KEY_FILE
```

Backup coordinates SQLite snapshots and durable blobs and writes a manifest last. Received cache and public models are excluded. Restore requires a new directory, clears authentication sessions/login flows, disables jobs and changes in-flight submissions to `delivery_unknown`. Inspect and reconcile before enabling jobs. Protect completed backups and separately back up encryption, identity-provider and VAPID keys. Encrypt off-host archives using operator-controlled tooling and test restore into an isolated directory.

Do not run an older binary directly against migrated data. Roll back using a matching restored snapshot and retain the newer data until reconciliation is complete. Local backup/restore behavior is covered by storage tests; this repository does not claim an operator's live disaster-recovery acceptance.

### Native backend release builder

Use `Dockerfile.build` when compiling Linux binaries in a container. It combines
Rust 1.98 with the Ubuntu 24.04 C runtime used by the production image. Building
against Debian Bookworm's older glibc can fail on `__isoc23_strtol` symbols from
the pinned inference library. Do not change runtime security or inference
features to work around a mismatched build environment.

Build the image using only its Dockerfile as context:

```sh
docker build -t dakia-release-builder - < deploy/Dockerfile.build
docker run --rm -v "$PWD:/source" -w /source dakia-release-builder \
  cargo build --release --locked -p dakia-server
```

Run this on a native Linux amd64 host. Mount a source checkout and optional
isolated Cargo/target caches only. Never mount production data or credentials
into the builder. Package all three binaries and matching ONNX runtime libraries,
and run the release gate before activation. Back up the stopped app before
changing the production image, retain its previous tag, and verify database
integrity, mounts, health and unchanged account/submission counts afterwards.
