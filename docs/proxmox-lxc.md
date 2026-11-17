# Install Dakia with the community-scripts framework

Dakia uses the [community-scripts](https://github.com/community-scripts/ProxmoxVE) two-file format:

- [`ct/dakia.sh`](../ct/dakia.sh) loads the shared framework, opens its container settings menus, and selects the latest stable Dakia release.
- [`install/dakia-install.sh`](../install/dakia-install.sh) runs inside the new guest and installs the checksum-verified native bundle.

This adapter lives in Dakia's repository. It is not an officially listed community-scripts application. It uses the shared engine pinned to a reviewed commit; the framework remains separately maintained and licensed by community-scripts.

## Run on Proxmox

Review the scripts and run from a checkout as root on the Proxmox host:

```sh
bash ct/dakia.sh
```

Enter the exact HTTPS origin when prompted, then choose **Default Install** or **Advanced Install** in the community-scripts menu. You can supply the origin beforehand:

```sh
var_dakia_origin=https://mail.example.com bash ct/dakia.sh
```

Defaults are Ubuntu 24.04, 2 CPU cores, 4 GiB RAM, a 20 GiB disk, and an unprivileged container. The framework handles template downloads, storage and bridge selection, networking, progress output, logging, and failure cleanup. Other guest operating systems are rejected because the native runtime targets Ubuntu 24.04. Framework diagnostics, host settings, and container customization follow community-scripts behavior.

For unattended creation, provide the framework settings explicitly:

```sh
PHS_SILENT=1 mode=default \
var_ctid=200 var_container_storage=local-lvm var_template_storage=local \
var_dakia_origin=https://mail.example.com bash ct/dakia.sh
```

An explicitly requested occupied guest ID is refused. The interactive framework otherwise selects an available ID. To keep a failed new guest for debugging, use the framework's `dev_mode=keep` option.

The framework can use local engine and script checkouts through `COMMUNITY_SCRIPTS_CORE_DIR` and `COMMUNITY_SCRIPTS_ROOT`. Remote script downloads default to Dakia's repository; `COMMUNITY_SCRIPTS_URL` can select another branch. `COMMUNITY_SCRIPTS_CORE_URL` can override the pinned engine URL. Running this adapter trusts both the shared framework code and the Dakia release publisher.

## What gets installed

The host checks GitHub's latest stable release for the architecture's native archive and `SHA256SUMS`. Inside the guest, the service user downloads both and verifies exactly one matching checksum before root extraction. The supplied launcher and systemd unit run the bundled Node, workers, web assets and models. No compiler, global Node/npm installation, or Docker is needed.

Application files are root-owned. Mail data and the credential key have separate private directories. The service starts and must pass its health check.

Dakia listens only on `127.0.0.1:3025` **inside the guest**. Configure a private HTTPS proxy or outbound tunnel inside the guest, create your owner login, and only then widen access. The installer doesn't configure DNS, public routing, mailboxes, or an owner account. An external proxy can't reach the guest's loopback address.

## Upgrade an existing LXC

Run the same launcher **as root inside the existing Dakia guest**, using a current checkout:

```sh
bash ct/dakia.sh
```

The framework dispatches to [`scripts/upgrade-native.py`](../scripts/upgrade-native.py). It resolves GitHub's latest stable release once, downloads the native bundle, verifies its SHA-256 checksum and version, and stages it while Dakia is still running. An equal or older release does nothing: no restart and no new backup.

For a newer release, the updater stops Dakia and uses the installed `dakia-admin` to make an offline snapshot. It also saves the credential key, configuration, referenced secret files, effective application settings, and systemd unit under `/var/lib/dakia/backups/`. Each backup is private to root and marked `COMPLETE` only after every part succeeds. Keep backups private and copy them to encrypted storage off the host. The updater retains backups and old releases; it does not prune them.

Only then does it switch the release symlink, install the new bundled unit, and start Dakia. It checks health, running binary identity, and that the credential key is unchanged. Expect downtime during the backup and startup. Allow disk space for the download, extracted release, and a full backup; a failed download or backup cannot activate the new release.

If a failure occurs before activation, the unchanged release restarts. If activation has begun, the updater stops Dakia and leaves `/var/lib/dakia-upgrades/active.json` pointing to the recovery backup. An interrupted attempt also blocks further upgrades. Follow [native recovery](native-hosting.md#updates-and-recovery), restore the matching snapshot and key into fresh directories, and reconcile uncertain sends before enabling jobs. Never switch the old runtime onto potentially migrated data. Remove the interruption marker only after recovery is complete; inspect any leftover release directory before retrying.

Automatic upgrades support the standard local SQLite layout, credential key location, service user, and bundled systemd unit on Ubuntu 24.04. Custom units, runtime paths, data/key locations, external databases, and pending configuration changes are refused before stopping Dakia. Use the manual guide for those installations. Apply configuration changes with a service restart before upgrading.

The guest installer continues to refuse replacement of an existing installation. The old `scripts/setup-proxmox-lxc.sh` path forwards to the menu launcher, but its former command-line flags are no longer supported.
