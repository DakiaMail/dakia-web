# Operate Dakia

Start with [hosting](../docs/hosting.md) if you haven't installed Dakia. These commands assume the user-owned directory from that guide and standard Linux Docker permissions:

```sh
cd ~/dakia
```

If your Docker installation works without sudo, omit `sudo` from Docker commands. If you used the root installer, use `sudo -i` and `cd /opt/dakia` instead. Keep the same directory, project name, `.env`, and Compose overrides for every operation. Plain Compose commands load `compose.override.yaml` automatically.

## Check status and logs

```sh
sudo docker compose ps
sudo docker compose logs --tail 100 dakia
sudo docker compose logs --tail 100 caddy
```

The last command applies only when Caddy is in your stack. Dakia should show `healthy`. Caddy has no supplied health check; confirm the actual HTTPS address in your browser. From a computer allowed through your firewall:

```sh
curl -fsS https://mail.example.com/healthz
```

Replace the example address. The result should identify `dakia-web` with status `ok`. A valid certificate, successful login, and mailbox access are separate checks. Logs and resolved configuration can contain private information; inspect them locally before sharing.

## Change configuration

Edit `.env` or your existing `compose.override.yaml`, then apply it:

```sh
sudo docker compose config --quiet
sudo docker compose up -d --wait
```

A restart alone doesn't apply changed environment variables. Don't replace your whole override with a provider example: merge its `dakia` settings alongside existing services and mounts. Keep `.env` mode 600 and its directory private. See [configuration reference](#configuration).

## Backup and restore

The Docker commands below use default SQLite storage. PostgreSQL offline tools also need the source `DAKIA_DATABASE_URL` and database network access; use the [database transfer guidance](#database-transfers) for that backend.

Back up regularly, not only when upgrading. A backup contains private mail and credentials. Keep an encrypted copy off the host, retain the matching image, and test restoration. Never copy a live SQLite directory as your only backup, and never use `docker compose down -v` on an installation you want to keep.

The following procedure is for the supplied `/data` and `/secrets` mounts. If you added OIDC, provider secret-file mounts, private VAPID configuration, or other extra bind mounts, save those original files privately too. Don't print them or commit them to Git. Keep backup storage outside the repository.

### Make a backup

Run from your installation directory in one shell. Stop if any command fails. First identify the **running** image, rather than assuming the Compose file still selects it:

```sh
container=$(sudo docker compose ps -q dakia)
test -n "$container"
image=$(sudo docker inspect --format '{{.Image}}' "$container")
mkdir -p -m 700 "$HOME/dakia-backups"
backup="$HOME/dakia-backups/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -m 700 "$backup"
printf '%s\n' "$image" > "$backup/image-id"
cp .env compose.yaml "$backup/"
if [ -f compose.override.yaml ]; then cp compose.override.yaml "$backup/"; fi
mkdir -m 700 "$backup/deploy"
cp deploy/Caddyfile "$backup/deploy/"
sudo docker image tag "$image" "dakia-backup:$(basename "$backup")"
```

The private `image-id` file and backup tag identify the matching local image. Retain it or export it for off-host recovery; an image ID isn't a downloadable registry reference. The saved Compose files and `.env` retain the configured registry pin.

Stop only Dakia. Caddy can keep running. Prepare private key and snapshot directories, then use that same image's offline backup tool:

```sh
sudo docker compose stop dakia
sudo docker run --rm --network none --read-only --tmpfs /tmp --user 0 \
  --volumes-from "$container":ro \
  --mount "type=bind,src=$backup,dst=/backup" \
  --entrypoint /bin/sh "$image" -c \
  'mkdir -m 700 /backup/secrets /backup/store && cp -a /secrets/. /backup/secrets/ && chown 1000:1000 /backup/store'
sudo docker run --rm --network none --read-only --tmpfs /tmp --user 1000:1000 \
  --volumes-from "$container" \
  --mount "type=bind,src=$backup/store,dst=/backup" \
  --entrypoint /app/dakia-admin "$image" \
  backup /data /backup/snapshot /secrets/credential-key
sudo docker compose start --wait dakia
```

Look for **Backup completed** before treating the backup as usable. The app data mount is writable because the offline tool takes a lock and may checkpoint SQLite. The server must be stopped for this entire operation. Root inside the first container copies only the explicitly mounted files; no Docker socket is mounted.

If backup fails, retain its files and inspect the error. Restart the unchanged app with `sudo docker compose start --wait dakia` when ready; don't change image versions as part of recovering a failed backup. Some snapshot files are owned by UID 1000, so use administrator access when copying the whole backup to encrypted off-host storage. Verify that copy includes `store/snapshot/manifest.json`, `secrets`, deployment files, and any extra secret files.

### Test a restore without replacing your mail

Set `backup` to the completed backup's absolute path if you're in a new shell. Use a fresh output directory for each attempt:

```sh
image=$(cat "$backup/image-id")
mkdir -p -m 700 "$HOME/dakia-restores"
restore="$HOME/dakia-restores/$(basename "$backup")"
mkdir -m 700 "$restore"
sudo docker run --rm --network none --read-only --tmpfs /tmp --user 0 \
  --mount "type=bind,src=$backup,dst=/backup,readonly" \
  --mount "type=bind,src=$restore,dst=/restore" \
  --entrypoint /bin/sh "$image" -c \
  '/app/dakia-admin restore /backup/store/snapshot /restore/data /backup/secrets/credential-key && cp -a /backup/secrets /restore/secrets && chown -R 1000:1000 /restore/data /restore/secrets'
```

Look for **Restore completed with jobs disabled**. Restored files live outside the completed backup so the backup stays separate from future app writes. This test doesn't stop or replace your running installation, doesn't contact providers, and doesn't start the restored app. If it fails, retain the output and choose another new directory after investigating.

### Activate restored data

For actual recovery, stop Dakia first. Preserve the failed data. In your installation directory, create `compose.restore.yaml` using the matching image from `image-id` and the absolute restore paths:

```yaml
services:
  dakia:
    image: "REPLACE_WITH_CONTENTS_OF_IMAGE_ID"
    pull_policy: never
    volumes:
      - /absolute/path/to/restored/data:/data
      - /absolute/path/to/restored/secrets:/secrets
```

For the standard Caddy setup, validate and start with all three files:

```sh
sudo docker compose stop dakia
sudo docker compose -f compose.yaml -f compose.override.yaml -f compose.restore.yaml config --quiet
sudo docker compose -f compose.yaml -f compose.override.yaml -f compose.restore.yaml up -d --wait --pull never
```

If you don't have `compose.override.yaml`, omit that `-f` pair. Include any other overrides you normally use. Keep the original origin and identity-provider settings. Use this exact file selection for later commands until you have deliberately made these recovery mounts and image your normal configuration. A plain `docker compose up` would select the old mounts again.

Sign in again and inspect the restored accounts and mail. Restores invalidate sessions and invitations, leave background jobs disabled, and mark interrupted submissions as delivery-unknown. Reconcile uncertain sends with the provider before enabling jobs; a failed Sent filing isn't a reason to resend. Restore cannot undo mail actions already performed at the provider.

After reconciliation, stop the restored app and explicitly enable jobs with its matching image and mounts:

```sh
sudo docker compose -f compose.yaml -f compose.override.yaml -f compose.restore.yaml stop dakia
sudo docker compose -f compose.yaml -f compose.override.yaml -f compose.restore.yaml run --rm --no-deps \
  --entrypoint /app/dakia-admin dakia enable-jobs /data /secrets/credential-key --reconciled
sudo docker compose -f compose.yaml -f compose.override.yaml -f compose.restore.yaml up -d --wait --pull never
```

Again omit or add override files to match your installation. Never start an older image on newer, migrated data. Keep the original data and backup until recovery is accepted.

## Upgrades

See [upgrade and recovery](../docs/upgrading.md). Keep the previous image and a matching backup. Published image identity and maintainer builds are documented in [building and publishing](building.md).

## Troubleshooting

| Symptom | Next step |
| --- | --- |
| Docker says permission denied | Use the documented `sudo docker` commands. Don't make the Docker socket world-writable. |
| Port 80 or 443 is already in use | Keep your existing proxy and use the existing-proxy path. Don't stop unrelated services. |
| App is healthy but HTTPS fails | Check Caddy logs, DNS A/AAAA records, provider firewall rules, and certificate issuance. Don't disable certificate checks. |
| Sign-in returns to the login page | Use the exact `DAKIA_ORIGIN`, including any nonstandard port. Preserve Origin through the proxy. |
| Provider settings aren't applied | Merge the settings into the loaded override, then run `config --quiet` and `up -d --wait`. A restart alone isn't enough. |
| A mailbox needs reconnection | Check its provider credentials and permissions in Workspace. For IMAP/SMTP, check outbound connectivity; Microsoft uses outbound HTTPS. |
| Upgrade helper finds no published release | It doesn't offer arbitrary main-branch images. Keep your current installation or follow the reviewed manual image procedure. |
| Data appears empty after recreation | Stop and inspect the Compose project name and `/data` and `/secrets` mounts before creating another owner. Don't delete volumes or regenerate keys. |

## Configuration

The root image already supplies the data, key, web, worker, and model paths. For the supplied Compose package, set `DAKIA_ORIGIN` in `.env`; `DAKIA_PORT` optionally changes the host port. Neither setting is a password.

| Variable | Root image default / purpose |
| --- | --- |
| `DAKIA_ORIGIN` | Required external HTTPS origin, without a path or trailing slash |
| `DAKIA_DATA_DIR` | `/data`; persistent local data, SQLite account files and blobs |
| `SQLITE_TMPDIR` | `/data`; disk-backed SQLite temporary files for large upgrades and queries |
| `DAKIA_DATABASE_URL` | Optional application backend; unset/empty keeps SQLite; PostgreSQL uses native control/account schemas |
| `DAKIA_SEARCH_DATABASE_URL` | Optional independent search backend; unset/empty follows application storage |
| `DAKIA_CREDENTIAL_KEY_FILE` | `/secrets/credential-key`; exactly 32 raw bytes, generated on a fresh installation if absent |
| `DAKIA_OIDC_ISSUER` | Unset for local login; exact HTTPS issuer when using OIDC |
| `DAKIA_OIDC_CLIENT_ID` | Required with OIDC; confidential client with PKCE and callback `<origin>/auth/callback` |
| `DAKIA_OIDC_SECRET_FILE` | Required with OIDC; readable private client secret file |

A standalone server outside the supplied image needs explicit data and key paths. On first start, the key's parent directory must exist and be writable. Missing keys on an existing installation fail startup instead of being replaced.

Compose passes only the variables declared in its configuration. The supplied file already forwards origin, database settings, provider OAuth credentials, release checks, and administrator IDs. Other runtime settings need explicit `environment` entries and any mounts. Adding a value to `.env` alone doesn't forward it.

For OIDC, merge the following `dakia` service settings into `compose.override.yaml`, retaining any existing Caddy service. Set the issuer, client ID, and host secret-file path in `.env`. Put `dakia` under the existing top-level `services`, alongside `caddy`; keep the top-level `volumes` section separate. Use one `services` key. Plain Compose commands load the override automatically; explicit `-f` commands must name it too:

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
| `DAKIA_WEB_ROOT` | Image: `/app/web`; standalone: `apps/web/dist` |
| `DAKIA_DURABLE_ATTACHMENT_BUDGET_BYTES` | 1073741824 per account; positive integer |
| `DAKIA_WORKERS` | Node worker source/dependency directory |
| `DAKIA_NODE` | Node executable, default `node` |
| `DAKIA_MODEL_CACHE` | Writable cache for public translation models |
| `DAKIA_VAPID_FILE` | Launchers default to `vapid.json` beside `DAKIA_CREDENTIAL_KEY_FILE` (`/secrets/vapid.json` in images). Unique private keys auto-provision on first start and persist across upgrades; existing configured keys are preserved |
| `DAKIA_VAPID_SUBJECT` | First-start HTTPS/mailto contact, defaults to `DAKIA_ORIGIN`. Does not replace the subject in an existing key file |

Native and Compose setup enable server-side notification capability by default without a paid push service. The private persistent secrets directory must be writable on first start; subsequent starts validate and preserve the keys. Keep the VAPID file in backups. Invalid configuration or provisioning failures stop startup rather than reporting false readiness. Device permission and per-device subscriptions still require user action. See [workers/README.md](../workers/README.md). These defaults take effect in runtimes built with the new launchers, not older pinned images.

Preserve key bytes across upgrades. Restrict secret directory/file permissions. New local setup needs writable `/secrets` to create its key. Existing provisioned secret files, such as an OIDC client secret, can be mounted read-only. Never put credentials in command arguments, public URLs, Git or logs. Treat database connection URLs and populated environment files as secrets. Give the service UID access to its data/model cache, not other application stores. Keep live SQLite on a local filesystem, not a network share. Run only one server/operator against a data directory; an exclusive lock enforces this.

## Local users

The first login owns the installation. Workspace provides owner-only invitation creation and revocation. Invitations work once and expire after 24 hours. See [getting started](../docs/getting-started.md) for connecting mailboxes and joining by invitation.

## Translation administration

The owner chooses the translation location in Workspace → Translation. OIDC operators can nominate stable Dakia user IDs with `DAKIA_ADMIN_USERS`. Server translation is the default; on-device mode downloads language packs to each browser when requested. See [translation settings](../docs/on-device-translation.md).

## Diagnostics

Private bounded mail diagnostics live under `DAKIA_DATA_DIR/diagnostics`. Keep Docker stdout rotation bounded too. Preserve logs privately before replacing a failing container. Don't post raw mail, tokens, or credentials in public issues.

IMAP failures back off per account, initially for five seconds and up to five minutes. A successful connection doesn't establish that a mail operation succeeded. Inspect diagnostics and provider state before retrying uncertain writes. Worker checks and image smoke commands belong to the [maintainer build guide](building.md).

## Database transfers

Backup coordinates portable control/account snapshots and durable blobs. PostgreSQL snapshots use the same SQLite archive format. Derived search text, received cache and public models are excluded.

For a backend transfer, use the same credential key and a build containing the
dual-database implementation. Set `DAKIA_DATABASE_URL` to the stopped source for
export, then to the new destination for import. An unset value selects SQLite.
The currently published image is not proof of this work-in-progress feature.

```sh
dakia-admin export DATA_DIR NEW_ARCHIVE_DIR KEY_FILE
dakia-admin import ARCHIVE_DIR NEW_DATA_DIR KEY_FILE
```

Import preserves IDs, ciphertext, revisions, queue ordering and durable jobs; it
pauses jobs without changing submission states. Search is rebuilt through normal
background acquisition. `restore` remains the disaster-recovery path and resets
authentication and uncertain sends as described above. PostgreSQL offline commands
hold an exclusive database-wide lease, so all online instances must be stopped,
even when they use other data directories or hosts. A populated target is refused.
Do not reuse a failed import directory. Its `backend-import-in-progress` marker
and PostgreSQL `backend_import=incomplete` setting prevent startup with partial
state. Preserve that evidence and inspect the isolated destination before retrying
into a fresh database and directory. Do not remove the guards to force startup.

See [database and search storage](../docs/dual-database-search.md) for backend details.
