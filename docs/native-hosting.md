# Run Dakia without Docker

Download a complete Linux runtime from [Releases](https://github.com/DakiaMail/dakia-web/releases). It includes Dakia, Node.js, the web app, workers, and search models. You don't need Rust, npm, or Docker.

These steps target Ubuntu 24.04 on AMD64 or ARM64, including an unprivileged LXC with systemd and persistent local storage. Other distributions haven't been checked. Keep your email provider and existing MX records.

For Proxmox, the [community-scripts launcher](../ct/dakia.sh) automates installation and upgrades with an offline backup first. See [Proxmox setup](proxmox-lxc.md).

## Before you start

You need administrator access, outbound HTTPS and mail-provider connections, and an HTTPS address routed through a reverse proxy. Dakia listens only on loopback.

The first login becomes the owner. Restrict your proxy or network firewall to your device **before** starting Dakia. Create your owner login before opening access to others. An unshared hostname isn't an access restriction. See [protect the first login](hosting.md#protect-the-first-login).

Install the runtime libraries and download tools:

```sh
sudo apt-get update
sudo apt-get install -y ca-certificates curl libssl3t64 libstdc++6 libatomic1
```

## Download and check the release

Choose a version from Releases. The example below installs `v0.1.0`. Run these commands as your normal user in a new directory, and stop if a command fails:

```sh
mkdir -m 700 dakia-download
cd dakia-download
version=v0.1.0
case "$(uname -m)" in
  x86_64) arch=amd64 ;;
  aarch64) arch=arm64 ;;
  *) echo 'Unsupported architecture'; exit 1 ;;
esac
archive="dakia-$version-linux-$arch.tar.gz"
base="https://github.com/DakiaMail/dakia-web/releases/download/$version"
curl -fL "$base/$archive" -o "$archive"
curl -fL "$base/SHA256SUMS" -o SHA256SUMS
awk -v archive="$archive" '$2 == archive' SHA256SUMS > "$archive.sha256"
test -s "$archive.sha256"
sha256sum -c "$archive.sha256"
```

Continue only if the checksum reports **OK**. Checksums detect damaged downloads; you still trust the release publisher.

## Install the service

The following is for a fresh installation. Don't run it over an existing Dakia installation or reuse its data with a different key.

```sh
sudo useradd --system --home-dir /var/lib/dakia --shell /usr/sbin/nologin dakia
sudo install -d -m 755 /opt/dakia/releases
sudo install -d -m 700 /etc/dakia
sudo install -d -o dakia -g dakia -m 700 /var/lib/dakia/data /var/lib/dakia-secrets
sudo mkdir "/opt/dakia/releases/$version"
sudo tar -xzf "$archive" -C "/opt/dakia/releases/$version"
sudo chown -R root:root "/opt/dakia/releases/$version"
sudo chmod -R go-w "/opt/dakia/releases/$version"
sudo ln -s "releases/$version" /opt/dakia/current
sudo install -m 644 /opt/dakia/current/dakia.service /etc/systemd/system/dakia.service
sudo install -m 600 /dev/null /etc/dakia/dakia.env
sudoedit /etc/dakia/dakia.env
```

Put the following in `dakia.env`, replacing the example origin with the exact HTTPS address people will use:

```ini
DAKIA_ORIGIN=https://mail.example.com
DAKIA_DATA_DIR=/var/lib/dakia/data
DAKIA_CREDENTIAL_KEY_FILE=/var/lib/dakia-secrets/credential-key
```

The supplied launcher selects the bundled Node, workers, models, and libraries. It keeps SQLite temporary files in the persistent data directory. The service runs as the `dakia` user with a read-only system filesystem, private temporary storage, and persistent write access to its data and secrets. The credential key is generated privately on first start.

Server-side Web Push is also ready by default: before server startup the launcher creates unique mode-0600 VAPID keys at `/var/lib/dakia-secrets/vapid.json`, using your HTTPS `DAKIA_ORIGIN` as the contact subject. The keys are owned by `dakia` and preserved on subsequent starts. Existing `DAKIA_VAPID_FILE` and its valid keys are honored without rotation; `DAKIA_VAPID_SUBJECT` optionally overrides the first-start contact subject. Back up the VAPID file with the other secrets. Invalid keys or provisioning failures stop startup with a sanitized error. See [worker configuration](../workers/README.md). Device permission and subscriptions remain explicitly user-controlled; no notification is sent during setup.

Set up your HTTPS reverse proxy before opening public access. For [Caddy](https://caddyserver.com/docs/install), use:

```caddyfile
mail.example.com {
    reverse_proxy 127.0.0.1:3025
}
```

Validate and reload the proxy without replacing unrelated sites. Keep your initial access restriction until owner setup is complete. Then start Dakia:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now dakia
sudo systemctl status dakia --no-pager
curl -fsS http://127.0.0.1:3025/healthz
```

Open your HTTPS address without bypassing certificate warnings. Choose **Sign in**, create your owner username and password, then sign out and back in. Connect a mailbox, open a message, and send a test message to an address you control. Only then widen access. See [getting started](getting-started.md) for provider setup and invitations.

Provider configuration belongs in `/etc/dakia/dakia.env`. Secret files must be readable by the service user; keep them in the private secrets directory. The [configuration reference](../deploy/README.md#configuration) describes available variables. Apply changes with `sudo systemctl restart dakia`.

## Updates and recovery

Keep `/var/lib/dakia` and `/var/lib/dakia-secrets` across upgrades. Never replace or regenerate the key for existing data. Backups contain private mail and credentials.

For a standard Proxmox LXC installation, re-run `bash ct/dakia.sh` as root **inside the guest** from a current checkout. It downloads and checksum-verifies the latest stable native release, backs up the stopped application, then activates and checks it. See [LXC upgrades](proxmox-lxc.md#upgrade-an-existing-lxc) for supported layouts and failure handling. The following manual procedure also covers customized native installations; external databases need their own consistent backup procedure.

Before upgrading, stop the service and create an offline snapshot with the **installed** runtime. Run the following in one shell, stopping if any command fails:

```sh
backup="/var/lib/dakia/backups/$(date -u +%Y%m%dT%H%M%SZ)"
sudo systemctl stop dakia
sudo install -d -o dakia -g dakia -m 700 "$backup"
sudo -u dakia env LD_LIBRARY_PATH=/opt/dakia/current \
  /opt/dakia/current/dakia-admin backup \
  /var/lib/dakia/data "$backup/snapshot" /var/lib/dakia-secrets/credential-key
sudo install -d -m 700 "$backup/secrets"
sudo cp -a /var/lib/dakia-secrets/. "$backup/secrets/"
sudo cp -a /etc/dakia "$backup/config"
sudo cp /opt/dakia/current/VERSION /opt/dakia/current/REVISION "$backup/"
```

Confirm **Backup completed** and retain the old release directory. Keep an encrypted copy of the snapshot, matching key, configuration, and release off the host. If backup fails, keep the output and restart the unchanged service; don't change versions while recovering from a failed backup.

Download and verify the new release using the steps above. Extract it into a **new** release directory, apply root ownership and remove group/other write permissions as above, then change only the `/opt/dakia/current` symlink while the service is stopped. Review any changes to the supplied service file before replacing your installed unit. Start Dakia and check health, HTTPS, sign-in, and mailbox access again. Restart the host or LXC once to verify automatic startup and persistence.

Don't start an older runtime against newer migrated data. Recovery needs the old runtime and a matching restored snapshot in new directories. The offline commands are:

```sh
sudo env LD_LIBRARY_PATH=/opt/dakia/releases/OLD_VERSION \
  /opt/dakia/releases/OLD_VERSION/dakia-admin restore \
  /absolute/path/to/snapshot /absolute/path/to/fresh-restored-data \
  /absolute/path/to/matching/credential-key
```

Restore leaves jobs disabled. Preserve failed data, make the restored files writable only by the service user, and point the service's data and key settings at the restored directories. Reconcile uncertain sends with your provider before using `dakia-admin enable-jobs DATA KEY --reconciled` with the matching runtime and the service stopped. Never resend just because Sent filing failed. The Docker upgrade helper doesn't manage native installations.

For logs, use `sudo journalctl -u dakia --since '1 hour ago'`. Logs and backups can contain private information; don't post raw output publicly. Configure host journal retention and routine backups before relying on the installation.
