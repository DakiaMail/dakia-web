# Host Dakia

Run Dakia with Docker Compose, or follow the [native Linux setup](native-hosting.md) to use a complete release bundle without Docker. Neither path requires compiling Rust. You keep your existing email provider; there is no mail server, database service, or MX change to set up.

The steps below install the latest stable release, [v0.1.0](https://github.com/DakiaMail/dakia-web/releases/tag/v0.1.0), using the image digest pinned in `compose.yaml`. They do not run Dakia scripts on your host. You still trust the container image and Docker; review the [Dockerfile](../Dockerfile) and [Compose file](../compose.yaml) before running them.

## Before you start

You need:

- A Linux AMD64 or ARM64 host with persistent local storage.
- Docker Engine and its Compose plugin. On a fresh server, follow Docker's package-repository instructions for [Ubuntu](https://docs.docker.com/engine/install/ubuntu/#install-using-the-repository) or [Debian](https://docs.docker.com/engine/install/debian/#install-using-the-repository). Install Git with your operating system's package manager if needed.
- An HTTPS address. Choose a public domain with Caddy, an existing reverse proxy, or private access with Tailscale Serve.

Commands use `sudo docker` for a standard Linux Docker installation. If Docker already works without sudo, such as Docker Desktop or rootless Docker, omit `sudo` from Docker commands. Adding yourself to the Docker group grants root-level access; it isn't required for these steps. See [Docker permissions](https://docs.docker.com/engine/install/linux-postinstall/).

Check the prerequisites:

```sh
sudo docker version
sudo docker compose version
git --version
```

### Protect the first login

The first login owns the installation. Limit who can reach it **before** starting it.

For a public server, use your provider's network firewall to allow TCP 443 only from your current public IP until you have created the owner login. Allow TCP 80 for Caddy's certificate validation. Keep your SSH access allowed. Then open 443 to others after setup. An unshared hostname is not private, and Docker-published ports can bypass UFW rules; use the provider's firewall or an equivalent Docker-aware network policy.

If you cannot restrict public access, start with [private Tailscale access](#private-access-with-tailscale). Don't expose an unclaimed installation while you work through setup.

## Prepare the files

Run these as your normal user. Use a new directory so you don't replace an existing installation:

```sh
git clone --depth 1 https://github.com/DakiaMail/dakia-web.git ~/dakia
cd ~/dakia
chmod 700 .
(umask 077; printf '%s\n' 'DAKIA_ORIGIN=https://mail.example.com' > .env)
```

Replace `https://mail.example.com` with the exact HTTPS address people will open. Don't include a path, query, or trailing slash. Include the HTTPS port if it isn't 443. Keep `.env` private; it can later contain provider secrets.

Keep this directory and both Docker volumes through upgrades. Run later commands from here. A second installation needs a different Compose project name as well as its own ports, origin, and storage; a different directory alone doesn't isolate it because `compose.yaml` names the project `dakia`.

## Public HTTPS with Caddy

Point your domain's A record at the server. If you have an AAAA record, it must also reach this server. Ports 80 and 443 must be available for Caddy; if you already have a proxy, use the next section instead of stopping it.

Copy the supplied Caddy configuration into Compose's automatic override:

```sh
cp compose.caddy.yaml compose.override.yaml
cat compose.yaml compose.override.yaml deploy/Caddyfile
sudo docker compose config --quiet
sudo docker compose pull
sudo docker compose up -d --wait
sudo docker compose ps
```

Review the files before starting. The app publishes only `127.0.0.1:3025`; Caddy publishes 80 and 443 and obtains the certificate. The app runs as UID 1000 with a read-only root filesystem, dropped capabilities, and no Docker socket. `config --quiet` checks configuration without printing resolved secrets.

Leave `compose.override.yaml` in place. Plain `docker compose` commands load it automatically, including later provider-secret mounts you add to that file. Don't overwrite it with another example. Additional `dakia` settings belong under the existing top-level `services`, alongside `caddy`, not under the top-level `volumes`. Explicit `-f` commands load only the named files, so include every override if you use them.

## Use an existing reverse proxy

Skip the Caddy copy step. For Caddy running directly on the Docker host:

```caddyfile
mail.example.com {
    reverse_proxy 127.0.0.1:3025
}
```

Validate and reload your existing proxy configuration without replacing unrelated sites. Preserve the browser's Origin header, then start Dakia:

```sh
sudo docker compose config --quiet
sudo docker compose pull
sudo docker compose up -d --wait
```

A proxy in another container needs a shared private Docker network and upstream `dakia:3025`. A proxy in another VM needs a reachable private bind address restricted to that proxy. `localhost` inside another container or VM isn't Dakia's host. Don't publish port 3025 to the internet.

## Private access with Tailscale

Skip the Caddy copy step. Install and sign in to Tailscale on the Docker host, enable MagicDNS and HTTPS for your tailnet, and check any existing Serve configuration before adding a route:

```sh
tailscale serve --bg http://127.0.0.1:3025
tailscale serve status
```

Set `DAKIA_ORIGIN` in `.env` to the HTTPS origin shown by Serve, then run:

```sh
sudo docker compose config --quiet
sudo docker compose pull
sudo docker compose up -d --wait
```

Users need access to your tailnet and a Dakia login. On a shared tailnet, restrict initial access to your device until the owner login exists. Follow [Tailscale's Serve guide](https://tailscale.com/docs/features/tailscale-serve) if its command requests administrator access or the route isn't available.

## Finish setup

Open your configured HTTPS address. Certificate issuance can take longer than the app's health check; a healthy app doesn't prove HTTPS is ready. Don't bypass certificate warnings. If the page doesn't open, check [operations and troubleshooting](../deploy/README.md#troubleshooting).

Choose **Sign in**, create the first username and password, then follow [Connect an email account](getting-started.md#connect-an-email-account). Once the owner login exists, you can widen network access. Don't remove Dakia's normal login protection from your proxy.

Confirm you can sign out and back in. Connect a mailbox, open a message, and send a test message to an address you control. Take a [backup](../deploy/README.md#backup-and-restore) before relying on the installation.

## Storage and network access

| Mount | Contents |
| --- | --- |
| `/data` | SQLite databases, messages/cache, drafts, attachments, diagnostics, and public model caches |
| `/secrets` | Credential encryption key |

The supplied Compose file creates named volumes for both. They must survive container recreation. Never run `docker compose down -v` on an installation you want to keep. Back up data and secrets with restricted access and encrypted off-host copies. Losing the key prevents decryption of stored mailbox credentials.

The container runs as UID/GID 1000. If replacing named volumes with host directories or NAS datasets, create dedicated directories writable by that identity and keep the secrets directory private. Keep live SQLite on local storage; do not put its database files on an SMB/NFS mount from another machine.

Allow outbound HTTPS, IMAP, and SMTP connections. Dakia does not require inbound SMTP ports or MX changes. Provider connectivity and storage capacity matter as mailboxes grow; minimum hardware requirements are not yet established.

## Managed platforms

A managed container platform can run the published image without a shell on the host. Dakia needs the platform to provide all of the following:

| Requirement | Why |
| --- | --- |
| An HTTPS address with a valid certificate | Sign-in is bound to the exact origin; plain HTTP is not supported |
| Two persistent volumes, mounted at `/data` and `/secrets` | Restarts and redeploys must keep mail, the credential key, and their file permissions |
| One always-on instance with no scale-to-zero | Synchronization, scheduled sending, and an exclusive lock on the data directory |
| Outbound IMAP, SMTP submission, and HTTPS | Port 25 is not needed; use the provider's submission port, 465 or 587 |
| Enough memory for your mailbox workload | Classification uses local rules; server translation adds language-model memory use |

Set `DAKIA_ORIGIN` to the exact HTTPS origin the platform gives you, without a trailing slash. It is a plain setting, not a password.

Restrict access through the platform before the first start, then create the owner login before opening access to others. Knowing an unshared address is not an access restriction. If the platform cannot restrict a fresh installation, use a private deployment path instead.

Managed platforms do not run on the host, so `scripts/upgrade.sh` does not apply. Upgrade by deploying the new image digest and keeping both volumes, and confirm the platform's backup covers `/secrets` as well as `/data`.

If you use Gmail sign-in, register `$DAKIA_ORIGIN/api/accounts/gmail/callback` in your own Google client. Platform origin changes require updating both the Google client and `DAKIA_ORIGIN`.

### Check the platform before installing

Confirm persistent storage, outbound IMAP/SMTP access, backups, and an always-on instance with your platform. Free tiers and email-port restrictions vary. A service that only hosts static pages cannot run Dakia. [PikaPods](../deploy/pikapods.md) has a separate catalog handoff.

## Platform paths

The Compose package and native AMD64/ARM64 images have been checked, and a Docker VM under Proxmox has been exercised with separate users and disposable TLS mailboxes. The NAS paths below describe manual configuration; platform-specific acceptance and catalog packages are not yet complete. The [dated VM setup exercise](testing/self-hosting.md) records the latest tested path and its limits.

### PikaPods

Dakia is not yet a PikaPods catalog app. The [PikaPods catalog handoff](../deploy/pikapods.md) records the configuration and checks needed before it can be offered there.

### DigitalOcean or Hetzner Cloud

Create a Debian or Ubuntu server with Docker support and persistent local storage. Point your domain at it, then follow the [Compose steps](#prepare-the-files). The optional [cloud-init installer](#optional-installer) is available if you prefer automated host setup. Keep the server running for sync and scheduled sending, and back up both Dakia volumes.

### Memory planning

Current minimum hardware requirements haven't been established. Older measurements used a model-based classifier that Dakia no longer ships, so they aren't a sizing guide for the current app. Classification now uses deterministic rules without a model download.

Mailbox size, concurrent users, attachments, and translation affect resource use. On-device translation moves inference to each browser; server translation needs memory for its language models. Test representative mailboxes and watch memory and disk use before relying on the installation.

### Proxmox

1. Use a Debian or Ubuntu VM with persistent local disk storage.
2. Install Docker Engine and Compose inside that guest, following the [Docker installation guide](https://docs.docker.com/engine/install/).
3. Follow the Compose and HTTPS steps above inside the VM. If using a proxy in another VM, account for the loopback binding described above.
4. Keep the guest running for background work. Test backup and restore before depending on it.

A dedicated Proxmox appliance image is not supplied. Nested Docker in LXC is outside the tested path.

### Mac mini

Install a Docker runtime with Compose, then use the same instructions. Docker selects ARM64 on Apple Silicon or AMD64 on Intel. Configure the runtime to start after reboot and keep the Mac awake when syncing or scheduled sending is needed. Test that the service returns after a reboot. Native macOS server packaging is not supplied.

### Synology

On a supported model, create a [Container Manager Project](https://kb.synology.com/en-global/DSM/help/ContainerManager/docker_project) from `compose.yaml`. Supply the `.env` values, preserve both volumes, and configure the NAS's HTTPS proxy to the published loopback port. If using host directories, set permissions for UID/GID 1000. Check that your NAS supports one of the image architectures.

### TrueNAS

Use the [custom-app Compose YAML interface](https://www.truenas.com/docs/scale/apps/installcustomappscreens/). Supply the origin and image from the Compose package, preserve data and secrets in named volumes or dedicated local datasets, and grant UID/GID 1000 the required access. Configure HTTPS with a reachable private upstream; a separate proxy app cannot use Dakia's loopback address as its own localhost. No Dakia catalog entry is currently supplied by this repository.

### Unraid

For a manual Docker container, copy the full `ghcr.io/...@sha256:...` image reference from the default in `compose.yaml`. Map a persistent appdata directory to `/data` and a separate private directory to `/secrets`, configure `DAKIA_ORIGIN`, and route container port 3025 through your HTTPS proxy. Ensure UID/GID 1000 can write its directories. The repository does not yet supply a Community Applications template; do not assume a similarly named catalog item is this package.

### Portainer

Create a [stack](https://docs.portainer.io/user/docker/stacks/add) using the Compose file. Supply `DAKIA_ORIGIN` and optional `DAKIA_PORT` through stack environment variables or the `.env` loader. Keep the image pin and both volumes. Configure your proxy according to whether it runs on the host or in another container. No custom Portainer template is required.

## More than one person

Complete owner setup, then use Workspace to create invitations. See [inviting someone](getting-started.md#invite-another-person) and [joining an installation](getting-started.md#join-an-installation). A Dakia invitation does not change LAN, VPN, or proxy access rules.

## Existing OIDC installations

Keep the existing issuer, client ID, secret file, origin, credential key, and data mounts. A fully configured OIDC installation continues using its original login flow and user identities. Manage people through that identity provider; local invitations are disabled in OIDC mode.

Removing OIDC settings from an installation with OIDC users fails startup. It does not convert those people to local accounts. Local and OIDC identities are not automatically linked.

## Updates and recovery

Use the [upgrade guide](upgrading.md) to choose a new image and keep the same persistent volumes. The optional upgrade helper makes a backup before activation. Take [routine backups](../deploy/README.md#backup-and-restore) even when you are not upgrading.

The local owner also sees a dismissible release notice in the app. See [upgrading and recovery](upgrading.md) for existing-installation setup, preview releases, private backup storage, recovery, and manual NAS/Portainer upgrades.

Pulling the currently pinned image alone does not select a new version. Keep both persistent volumes and the credential key. Never start an older image against migrated data; recovery uses a matching restored snapshot in fresh storage.

## Optional installer

The main setup above doesn't need an installer. `scripts/install.sh` and [`deploy/cloud-init.yaml`](../deploy/cloud-init.yaml) remain available for operators who want automated host setup.

The installer runs as root, installs Docker through Docker's convenience script when Docker is missing, downloads deployment files, writes `/opt/dakia`, and starts Caddy on ports 80 and 443. Review both the installer and the Docker installation method before choosing it. Use it on a fresh Debian or Ubuntu server, with the same DNS and first-login network restrictions described above.

To inspect a downloaded copy without running it:

```sh
curl -fsSL -o dakia-install.sh https://raw.githubusercontent.com/DakiaMail/dakia-web/main/scripts/install.sh
less dakia-install.sh
```

`sh dakia-install.sh --help` lists its options. After review, `sudo sh dakia-install.sh --domain mail.example.com` starts installation. `--prepare-only` writes files without installing Docker or starting services, and can run without root in a directory you own.

The root installer creates a private, root-owned `/opt/dakia`. For maintenance, enter a root shell with `sudo -i`, then `cd /opt/dakia`. Its Caddy file is named `compose.caddy.yaml`; either include it explicitly in Compose commands or, before adding other overrides, copy it to `compose.override.yaml` so ordinary commands load it. Keep any provider and OIDC changes in that same override.
