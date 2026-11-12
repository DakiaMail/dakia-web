# Deployment and operations

For new installations, start with [hosting](../docs/hosting.md) and the root Compose package. The root Dockerfile targets Linux AMD64 and ARM64. The legacy bundle packaging below targets Linux x86_64/glibc 2.39 (Ubuntu 24.04). It produces a local runtime image, not a hosted service or an automatic deployment workflow. Use an HTTPS reverse proxy. Existing installations can continue using an operator-controlled OIDC provider; new installations can use built-in local login. Keep the application private until access, provider behavior and backups have been validated for your environment.

## Configuration

The root image already supplies the data, key, web, worker, and model paths. For the supplied Compose package, set `DAKIA_ORIGIN` in `.env`; `DAKIA_PORT` optionally changes the host port. Neither setting is a password.

| Variable | Root image default / purpose |
| --- | --- |
| `DAKIA_ORIGIN` | Required external HTTPS origin, without a path or trailing slash |
| `DAKIA_DATA_DIR` | `/data`; persistent control/account databases, mail and blobs |
| `DAKIA_CREDENTIAL_KEY_FILE` | `/secrets/credential-key`; exactly 32 raw bytes, generated on a fresh installation if absent |
| `DAKIA_OIDC_ISSUER` | Unset for local login; exact HTTPS issuer when using OIDC |
| `DAKIA_OIDC_CLIENT_ID` | Required with OIDC; confidential client with PKCE and callback `<origin>/auth/callback` |
| `DAKIA_OIDC_SECRET_FILE` | Required with OIDC; readable private client secret file |

A standalone server outside the supplied image needs explicit data and key paths. On first start, the key's parent directory must exist and be writable. Missing keys on an existing installation fail startup instead of being replaced.

The root Compose file passes `DAKIA_ORIGIN` to the container. Other runtime settings need explicit `environment` entries and any required mounts; simply adding them to `.env` does not pass them through automatically. For OIDC, save the following as `compose.override.yaml` beside `compose.yaml` and set its issuer, client ID, and host secret-file path in `.env`. Compose loads this override automatically:

```yaml
services:
  dakia:
    environment:
      DAKIA_OIDC_ISSUER: ${DAKIA_OIDC_ISSUER:?Set the issuer}
      DAKIA_OIDC_CLIENT_ID: ${DAKIA_OIDC_CLIENT_ID:?Set the client ID}
      DAKIA_OIDC_SECRET_FILE: /run/secrets/dakia-oidc
    volumes:
      - ${DAKIA_OIDC_SECRET_HOST_FILE:?Set a private absolute file path}:/run/secrets/dakia-oidc:ro
```

Configure all three OIDC settings together, and keep the actual secret outside the repository in a file readable by container UID 1000. Preserve existing OIDC/data/key configuration when upgrading. OIDC mode uses the identity provider's account management and disables local invitations; it does not link local and OIDC identities automatically.

Microsoft mailbox authorization uses an operator-owned Entra application and Microsoft Graph (not IMAP/SMTP). Set `DAKIA_MICROSOFT_CLIENT_ID`, `DAKIA_MICROSOFT_CLIENT_SECRET` (or preferably a private `DAKIA_MICROSOFT_SECRET_FILE` mount), and optionally `DAKIA_MICROSOFT_TENANT` (default `common`). See [Microsoft setup and limits](../docs/microsoft-oauth.md).

Gmail mailbox authorization uses a separate operator-owned Google OAuth client. Set `DAKIA_GMAIL_CLIENT_ID` and either `DAKIA_GMAIL_SECRET_FILE` (preferred) or `DAKIA_GMAIL_CLIENT_SECRET`. Configure both together; partial configuration fails startup. The supplied Compose file passes the ID and environment secret through. For the secret-file mount, Google console setup, and isolated development testing, see [Gmail OAuth](../docs/gmail-oauth.md).

Optional/runtime settings:

| Variable | Default or purpose |
| --- | --- |
| `DAKIA_LISTEN` | Image: `0.0.0.0:3025`; standalone: `127.0.0.1:3025`. Compose publishes only host loopback by default |
| `DAKIA_WEB_ROOT` | `apps/web/dist`; set `/app/web` in the packaged image |
| `DAKIA_DURABLE_ATTACHMENT_BUDGET_BYTES` | 1073741824 per account; positive integer |
| `DAKIA_WORKERS` | Node worker source/dependency directory |
| `DAKIA_NODE` | Node executable, default `node` |
| `DAKIA_MODEL_CACHE` | Writable cache for public translation models |
| `DAKIA_VAPID_FILE` | Private VAPID JSON file for Web Push; see `workers/README.md` |

Preserve key bytes across upgrades. Restrict secret directory/file permissions. New local setup needs writable `/secrets` to create its key and setup-code file. Existing provisioned secret files, such as an OIDC client secret, can be mounted read-only. Never put credentials in command arguments, URLs, Git or logs. Give the service UID access to its data/model cache, not other application stores. Keep live SQLite on a local filesystem, not a network share. Run only one server/operator against a data directory; an exclusive lock enforces this.

## Local users

The first local login is the owner. Workspace provides owner-only invitation creation and revocation. Invitations are single-use, expire after 24 hours, and allow a recipient to choose their own credentials and connect separate mailboxes. Existing local owners retain their ID, password hash, mailbox ownership, and sessions through the invitation migration. See the [user guide](../docs/getting-started.md).

## Published images

GitHub Actions builds application source changes on `main` for Linux AMD64 and ARM64. Source images use `ghcr.io/dakiamail/dakia-web:sha-<source-commit>` and carry the source commit in the `org.opencontainers.image.revision` label. Semantic version tags publish versioned images, then a GitHub release with a digest manifest and the paired upgrade scripts after packaged-runtime checks pass. Stable releases take priority over previews for update discovery. Manual workflow runs can build a chosen source revision.

Each native image must pass packaged classifier, translation-worker, health, Gmail OAuth, and restart checks before the multi-platform image is published. The classification check verifies the deterministic policy on synthetic messages as the normal app user. The OAuth checks use synthetic credentials with networking disabled. `edge` tracks a successful `main` build; the Compose default uses an explicitly verified digest. Documentation and Compose-pin-only changes do not rebuild unchanged application code.

Use the source-revision label and immutable digest to identify an image. Keep operator credentials and mail data in runtime configuration and persistent volumes, outside the image.

## Build

Install the requirements and run the checks in the root README. Retrieve the oversized model first:

```sh
sh deploy/build-bundle.sh
```

The builder installs locked npm dependencies, builds/tests the UI, tests the workers and Rust workspace, builds release binaries, and creates `deploy/bundle/runtime.tar.gz`. It packages copied binaries, web assets, workers and classification policy, excluding account data and operator secrets. Build on Linux x86_64 using Node 24 LTS and the pinned Rust toolchain. Classification requires no model files or runtime downloads.

Extract the archive into a new build directory and build the included Dockerfile with an explicit local tag. It expects `bin/`, `web/`, `workers/` beside it. The image runs as UID/GID 1000. Mount persistent storage at `/data`, set the required environment and mount private configuration files at their configured paths. Configure `DAKIA_WEB_ROOT=/app/web`; the image already sets the worker and translation model-cache paths. A private VAPID file is needed for push, not embedded in the image.

Use a read-only root filesystem, no new privileges, dropped capabilities and suitable resource limits. Networking must permit OIDC discovery/backchannel, mail provider TLS, selected model downloads and supported push endpoints. Do not bypass TLS verification to resolve routing failures. Ensure `/sw.js` serves JavaScript rather than the SPA fallback.

## Upgrade and verification

For supported Compose installations, use `sh scripts/upgrade.sh`. See [upgrading and recovery](../docs/upgrading.md) for release selection, backup locations, failure handling, and manual platform instructions. Automatic owner release checks can be disabled with `DAKIA_RELEASE_CHECKS=false`.

Keep the previous image and a matching backup before replacing the application. Retain the existing data and secret mounts, identity-provider state and proxy configuration. Never apply an old full proxy/compose file over unrelated operator changes.

Preserve incident stdout privately before replacement. Mail diagnostics are also retained in a bounded private ring under `DAKIA_DATA_DIR/diagnostics`, on the existing persistent mount. Keep Docker stdout rotation separately bounded. See the private diagnostics directory for correlation, uncertainty semantics, limits and privacy rules.

IMAP connection failures use an account-wide cooldown shared by command and IDLE lanes: 5 seconds initially, doubling to at most 5 minutes. Clean commands reset it; replacing account credentials creates a fresh provider. Cancelled handshakes also retain the cooldown. Diagnostics distinguish `connection_attempt` stage/outcome and safe transport reason, `connection_backoff` suppressed attempts, and `provider_backoff` cached/suppressed caller work. These are separate from checked wire commands. A successful authenticated connection alone does not prove a mail operation succeeded. The daily analyzer reports connection attempts and suppressions separately, without logging endpoints, credentials or reply content.


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

Stop the server before invoking the offline operator with its matching binary and key. Do not run it against a concurrently active store. The container includes `/app/dakia-admin`; override the image entrypoint when using it in a separate stopped-data operator container, and mount a backup directory writable by UID 1000. The commands below describe the binary arguments, not commands to run inside a live server container.

```sh
dakia-admin backup DATA_DIR NEW_BACKUP_DIR KEY_FILE
dakia-admin restore BACKUP_DIR NEW_DATA_DIR KEY_FILE
```

Backup coordinates SQLite snapshots and durable blobs and writes a manifest last. Received cache and public models are excluded. Restore requires a new directory, clears authentication sessions/login flows and pending invitation links, disables jobs and changes in-flight submissions to `delivery_unknown`. Inspect and reconcile before enabling jobs. Protect completed backups and separately back up encryption, identity-provider and VAPID keys. Encrypt off-host archives using operator-controlled tooling and test restore into an isolated directory.

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
into the builder. Package all three binaries without classifier model or runtime assets,
and run the release gate before activation. Back up the stopped app before
changing the production image, retain its previous tag, and verify database
integrity, mounts, health and unchanged account/submission counts afterwards.

### Translation administration

The local owner can choose translation location in Workspace → Translation. OIDC operators can nominate stable Dakia user IDs with `DAKIA_ADMIN_USERS` (comma-separated). Existing installations default to host translation. Public browser language packs use the existing writable `DAKIA_MODEL_CACHE`; mode changes persist in the control database. See [translation operations](../docs/on-device-translation.md).
