# Upgrade Dakia

Use the same installation directory, Compose project, `.env`, overrides, and persistent volumes. Take a [backup and test a restore](../deploy/README.md#backup-and-restore) first. Never start an older image against data already migrated by a newer one.

The supplied Compose file pins a published image by digest. That image can be newer than the latest tagged release. The helper below offers only published GitHub releases; it won't upgrade to an arbitrary source image.

## Change the image yourself

This path uses Docker commands without running a Dakia host script.

1. Choose a published image from the repository's [container packages](https://github.com/DakiaMail/dakia-web/pkgs/container/dakia-web). Check its source revision and successful native AMD64/ARM64 image workflow before choosing the immutable `ghcr.io/dakiamail/dakia-web@sha256:…` reference. If using a tagged release, its `dakia-release.json` asset supplies the image reference.
2. Back up with the currently running image as described in operations. Retain that image and backup together.
3. Set `DAKIA_IMAGE` in the existing private `.env` to the full immutable image reference you chose. Don't replace the file or change the origin, project name, keys, mounts, or provider settings.
4. Validate, pull, and recreate:

```sh
sudo docker compose config --quiet
sudo docker compose pull dakia
sudo docker compose up -d --wait
sudo docker compose ps
```

Open the HTTPS address, sign in, and check mailbox access. If the new version fails after opening the data, stop it and [restore the matching backup into fresh storage](../deploy/README.md#activate-restored-data). Changing the image pin back alone is not a safe rollback.

For image provenance, see [building and publishing](../deploy/building.md#published-images). You don't need to build an image yourself.

## Optional backup-and-upgrade helper

The helper automates release selection, backup, activation, and health checks. Read `scripts/upgrade.sh` and `scripts/upgrade.mjs` before running it. On standard Linux Docker it needs administrator access to Docker and private backup files; choosing it means trusting that host script. The manual path above avoids it.

From the user-owned installation directory:

```sh
sudo sh scripts/upgrade.sh --check
sudo sh scripts/upgrade.sh
```

For Docker Desktop or another installation that works without sudo, omit `sudo`. For the root installer, use `sudo -i`, then `cd /opt/dakia` before running the commands. The helper uses plain Compose commands, so keep Caddy and custom settings in the automatic `compose.override.yaml`. Convert a Caddy-only `compose.caddy.yaml` installation by copying it to `compose.override.yaml` before adding other overrides. Don't replace an existing override.

The helper requires a local Docker context. It rejects `COMPOSE_FILE`, `COMPOSE_ENV_FILES`, and `DOCKER_HOST` overrides and unsupported mounts before stopping the service. If your installation depends on custom file selection, use the manual path rather than unsetting variables to bypass these checks.

It shows release notes and asks before proceeding. It downloads the chosen image, briefly stops Dakia, backs up data and secrets, saves the new image digest in `.env`, and verifies startup. An unchanged image is a successful no-op. No published release means no upgrade and no change to the installation; it doesn't mean your setup is broken. Stable releases take priority over previews.

To select a particular published release or skip the confirmation after review:

```sh
sudo sh scripts/upgrade.sh --version v0.2.0
sudo sh scripts/upgrade.sh --yes
```

## Backups and interrupted helper upgrades

Backups are private directories under `.dakia-upgrades` in the installation directory. They include a completed data snapshot, credential secrets, supported external secret files, original deployment files, resolved configuration, image references, recovery instructions, and a progress record. These files contain private information. Keep them outside Git, restrict access, and make encrypted off-host copies with your own backup tooling. Nothing automatically deletes them. Ensure sufficient disk space for the snapshot and an additional restore before upgrading.

The previous image receives a local `dakia-upgrade-backup` tag. Keep that image and the backup together. Docker or filesystem errors before activation abort the upgrade. If backup fails after stopping Dakia, the unchanged container is restarted.

An interrupted or failed attempt leaves `.dakia-upgrades/active` pointing to its record. Inspect its `progress`, `backup.log`, and `start.log` privately. After an abrupt interruption, confirm the actual running image and whether the new version has opened the data. Once activation has begun, use recovery rather than starting an older image on the same data. Only remove `active` when the incident is resolved. A stale `lock` directory can be removed after confirming no upgrade command is running.

## Recover a failed upgrade

A failed startup stops the new container and prints the exact backup path. It keeps the failed data intact. First ensure the Dakia service is stopped, then run the generated recovery script:

```sh
sudo sh /absolute/path/to/backup/recover.sh
```

The script restores with the previous image into a new `restored/data` directory. It never replaces the failed data or starts the application. If restore fails, inspect the new directory before retrying; never merge partial restores. The script prints the exact Compose command for activating the restored directory, saved secrets, and previous image. Review it before running it.

Use that printed command for all subsequent Compose operations on the recovered installation. Its final `-f recovery.compose.json` selects the restored data and image. Ordinary `docker compose up` still selects the failed installation. Once recovery is accepted, move the recovery mounts/image into your normal deployment configuration, validate the resolved mounts, and retain both data copies until reconciliation is complete. Keep the same origin, identity provider, and any external proxy configuration.

Restores invalidate sessions and invitations and disable background jobs. Sign in again, inspect mailbox state, and reconcile uncertain sends with the provider before enabling jobs. Never retry SMTP delivery just because Sent filing failed. Restore cannot undo actions already performed at your mail provider.

To enable jobs after reconciliation, keep the complete Compose prefix printed by `recover.sh`, including its project, environment file, and every `-f` option. First replace the printed command's `up -d --no-deps --pull never dakia` suffix with `stop dakia`. Then use the same prefix with `run --rm --no-deps --entrypoint /app/dakia-admin dakia enable-jobs /data /secrets/credential-key --reconciled`. Finally run the original printed startup command. The [manual recovery example](../deploy/README.md#activate-restored-data) shows these operations with a complete file selection.

Then restart using the recovery Compose options. Early preview images may not include `enable-jobs`; keep jobs disabled until a matching operator is available. Do not use a newer operator against the restored store merely to enable jobs, because it can migrate the data. Restore preparation and read-only inspection remain available with those images.

## NAS dashboards and Portainer

Use the release's `dakia-release.json` asset to obtain the immutable `image` reference. Stop Dakia and make a backup with the installed version's matching operator binary, including the credential key and deployment settings. Change only the image reference in your container/project/stack settings, retain both persistent mounts, and redeploy. Pulling the existing digest alone does not select a release. Check container health, HTTPS, sign-in, and mailbox access.

If startup fails, stop the new version and restore a matching backup into fresh storage before using the previous image. See [operations](../deploy/README.md#backup-and-restore). Do not use `down -v` or delete application volumes.
