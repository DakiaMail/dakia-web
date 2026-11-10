# Upgrade Dakia

Run upgrades on the Docker host, from the same directory you used to install Dakia. Keep the project name, `.env`, Compose overrides, and persistent volumes. The command needs Docker Compose and a POSIX shell. 

```sh
sh scripts/upgrade.sh --check
sh scripts/upgrade.sh
```

The command shows the release notes and asks before proceeding. It downloads the release, briefly stops Dakia, backs up data and secrets, saves the new image digest in `.env`, and checks startup. An unchanged image is a successful no-op. Your owner login, mailbox connections, drafts, and credential key remain in the existing volumes.

Only published GitHub releases are offered. Stable releases take priority; previews are offered while no stable release exists. 

For a particular published release, or after reviewing the release in advance:

```sh
sh scripts/upgrade.sh --version v0.2.0
sh scripts/upgrade.sh --yes
```

## Backups and interrupted upgrades

Backups are private directories under `.dakia-upgrades` in the installation directory. They include a completed data snapshot, credential secrets, supported external secret files, original deployment files, resolved configuration, image references, recovery instructions, and a progress record. These files contain private information. Keep them outside Git, restrict access, and make encrypted off-host copies with your own backup tooling. Nothing automatically deletes them. Ensure sufficient disk space for the snapshot and an additional restore before upgrading.

The previous image receives a local `dakia-upgrade-backup` tag. Keep that image and the backup together. Docker or filesystem errors before activation abort the upgrade. If backup fails after stopping Dakia, the unchanged container is restarted.

An interrupted or failed attempt leaves `.dakia-upgrades/active` pointing to its record. Inspect its `progress`, `backup.log`, and `start.log` privately. After an abrupt interruption, confirm the actual running image and whether the new version has opened the data. Once activation has begun, use recovery rather than starting an older image on the same data. Only remove `active` when the incident is resolved. A stale `lock` directory can be removed after confirming no upgrade command is running.

## Recover a failed upgrade

A failed startup stops the new container and prints the exact backup path. It keeps the failed data intact. First ensure the Dakia service is stopped, then run the generated recovery script:

```sh
sh /absolute/path/to/backup/recover.sh
```

The script restores with the previous image into a new `restored/data` directory. It never replaces the failed data or starts the application. If restore fails, inspect the new directory before retrying; never merge partial restores. The script prints the exact Compose command for activating the restored directory, saved secrets, and previous image. Review it before running it.

Use that printed command for all subsequent Compose operations on the recovered installation. Its final `-f recovery.compose.json` selects the restored data and image. Ordinary `docker compose up` still selects the failed installation. Once recovery is accepted, move the recovery mounts/image into your normal deployment configuration, validate the resolved mounts, and retain both data copies until reconciliation is complete. Keep the same origin, identity provider, and any external proxy configuration.

Restores invalidate sessions and invitations and disable background jobs. Sign in again, inspect mailbox state, and reconcile uncertain sends with the provider before enabling jobs. Never retry SMTP delivery just because Sent filing failed. Restore cannot undo actions already performed at your mail provider.

With the restored service stopped, use its matching operator image and mounts to explicitly enable jobs after reconciliation:

```sh
docker compose <the printed recovery Compose options> run --rm --no-deps \
  --entrypoint /app/dakia-admin dakia \
  enable-jobs /data /secrets/credential-key --reconciled
```

Then restart using the recovery Compose options. Early preview images may not include `enable-jobs`; keep jobs disabled until a matching operator is available. Do not use a newer operator against the restored store merely to enable jobs, because it can migrate the data. Restore preparation and read-only inspection remain available with those images.

## NAS dashboards and Portainer

Use the release's `dakia-release.json` asset to obtain the immutable `image` reference. Stop Dakia and make a backup with the installed version's matching operator binary, including the credential key and deployment settings. Change only the image reference in your container/project/stack settings, retain both persistent mounts, and redeploy. Pulling the existing digest alone does not select a release. Check container health, HTTPS, sign-in, and mailbox access.

If startup fails, stop the new version and restore a matching backup into fresh storage before using the previous image. See [operations](../deploy/README.md#backup-and-restore). Do not use `down -v` or delete application volumes.
