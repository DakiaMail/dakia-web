# Hosting Dakia

Use the published container image with Docker Compose. It includes the web app, server, classifier, and workers; a source build and a separate database service are not required. The image supports Linux AMD64 and ARM64.

Start with [owner setup and everyday use](getting-started.md) after installation. For configuration, backups, and existing OIDC installations, see [operations](../deploy/README.md).

## Docker Compose

### One command on a fresh server

On a new Debian or Ubuntu server, point your domain's A record at it, keep ports 80 and 443 reachable, and run:

```sh
curl -fsSL -o dakia-install.sh https://raw.githubusercontent.com/DakiaMail/dakia-web/main/scripts/install.sh && sudo sh dakia-install.sh --domain mail.example.com
```

The script installs Docker, writes the Compose files and a private `.env` to `/opt/dakia`, starts Dakia with Caddy for automatic HTTPS, and waits for its health check. It needs a minute or two the first time. Then open your address and [create the first login](getting-started.md#create-the-first-login).

Use `--prepare-only` to write the files without installing Docker or starting anything, `--dir` to choose another directory, and `--help` for the rest. The same script is in [`deploy/cloud-init.yaml`](../deploy/cloud-init.yaml) for a Droplet or Hetzner Cloud server's user data, which needs no shell at all: replace the domain, create the server, and wait for HTTPS.

An installation made this way lives in `/opt/dakia`. Run upgrades there, keep its volumes, and use `docker compose -f compose.yaml -f compose.caddy.yaml` for every command.

### Manual

Install Docker with the Compose plugin on your host. Clone the repository and prepare configuration:

```sh
git clone https://github.com/DakiaMail/dakia-web.git
cd dakia-web
cp .env.example .env
```

Edit `.env` and set `DAKIA_ORIGIN` to the exact HTTPS origin people will open, for example `https://mail.example.com`. Include a nonstandard HTTPS port if you use one. Do not include a path, query, or trailing slash.

```sh
docker compose up -d --wait
docker compose ps
```

The default publishes `127.0.0.1:3025` on the Docker host. Configure one of the HTTPS options below, open your address, and follow [first login](getting-started.md#create-the-first-login). Ordinary users do not need Rust, Node.js, Python, or a registry login.

Run commands from the same directory and keep the same Compose project name. Changing the project name can select different volumes and make an existing installation appear empty. For an additional isolated instance, use a separate directory, project name, port, and origin; do not share its data or secrets volumes with another instance.

## HTTPS on the same host

### Caddy in the same Compose stack

The repository supplies a Compose override that runs Caddy in front of Dakia and obtains certificates for you:

```sh
docker compose -f compose.yaml -f compose.caddy.yaml up -d --wait
```

It reads `DAKIA_ORIGIN` from `.env`, serves that exact host on ports 80 and 443, and redirects HTTP to HTTPS. Certificates and Caddy's own state live in the `caddy_data` and `caddy_config` volumes; losing them only means reissuing certificates. Dakia keeps publishing `127.0.0.1:3025` for host-local access, so stop any other service that already holds ports 80 or 443.

Use this override for every later command, for example `docker compose -f compose.yaml -f compose.caddy.yaml logs -f dakia`. An upgrade still works from the installation directory with `sh scripts/upgrade.sh`.

### Existing reverse proxy

If Caddy runs directly on the same host as Docker, a site block can forward to Dakia:

```caddyfile
mail.example.com {
    reverse_proxy 127.0.0.1:3025
}
```

Set `DAKIA_ORIGIN=https://mail.example.com`. Configure DNS and certificate issuance for your network, validate your full proxy configuration, and reload it without replacing unrelated sites. See [Caddy's HTTPS reverse-proxy guide](https://caddyserver.com/docs/quick-starts/reverse-proxy).

The example assumes the proxy can reach the Docker host's loopback port. A proxy in another container needs a private Docker network and an upstream pointing to Dakia's container port 3025. A proxy in another VM needs a reachable private bind address and firewall rules that restrict access to the proxy. `localhost` in another container or VM does not refer to Dakia's host.

Preserve the browser's Origin header. A valid HTTPS certificate is required for normal sign-in; plain HTTP on a LAN address is not supported.

### Tailscale Serve

For private access, install and sign in to Tailscale on the Docker host, enable MagicDNS and HTTPS for your tailnet, and configure Serve on that host:

```sh
tailscale serve --bg http://127.0.0.1:3025
tailscale serve status
```

Set `DAKIA_ORIGIN` to the HTTPS origin shown by Serve, then run `docker compose up -d` again to apply it. Users need access to your tailnet as well as a Dakia login. This does not make the service public on the internet. Check existing Serve configuration before adding a route. See [Tailscale Serve](https://tailscale.com/docs/features/tailscale-serve) and its [command reference](https://tailscale.com/docs/reference/tailscale-cli/serve).

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
| At least 1 GB of memory | The classifier loads at boot; an idle installation used about 450 MB in testing |

Set `DAKIA_ORIGIN` to the exact HTTPS origin the platform gives you, without a trailing slash. It is a plain setting, not a password.

Create the first login immediately after the first deployment completes. A new installation accepts its first login from whoever reaches it and that login becomes the owner, so keep the address to yourself until you have created it. Platform-provided addresses are public by default.

Managed platforms do not run on the host, so `scripts/upgrade.sh` does not apply. Upgrade by deploying the new image digest and keeping both volumes, and confirm the platform's backup covers `/secrets` as well as `/data`.

If you use Gmail sign-in, register `$DAKIA_ORIGIN/api/accounts/gmail/callback` in your own Google client. Platform origin changes require updating both the Google client and `DAKIA_ORIGIN`.

### Platform notes

- **Fly.io, Render (paid), Zeabur** can run the image with a persistent volume and one always-on instance.
- **Railway** blocks outbound SMTP on its Hobby plan, so mail cannot be sent there. It needs Pro or higher.
- **DigitalOcean App Platform and Koyeb's free tier** have no persistent volume. They can show the interface but lose mail data and the credential key on every redeploy, so they are not suitable for an installation you want to keep. Use a Droplet or another advertised platform instead; the [Compose steps](#docker-compose) apply to a Droplet with Docker installed.
- **PikaPods** is tracked in the [catalog handoff](../deploy/pikapods.md).

## Platform paths

The Compose package and native AMD64/ARM64 images have been checked, and a Docker VM under Proxmox has been exercised with separate users and disposable TLS mailboxes. The NAS paths below describe manual configuration; platform-specific acceptance and catalog packages are not yet complete.

### PikaPods

Dakia is not yet a PikaPods catalog app. The [PikaPods catalog handoff](../deploy/pikapods.md) records the configuration and checks needed before it can be offered there.

### DigitalOcean or Hetzner Cloud

Create a Droplet or Cloud server from Ubuntu 24.04 with at least 1 GB of memory. If your region reports the 1 GB size as unavailable, choose its Intel or AMD variant; DigitalOcean capacity varies by region. Add your domain's A record first, then paste [`deploy/cloud-init.yaml`](../deploy/cloud-init.yaml) into the provider's user-data field with your domain in place of the example. The server installs Dakia and Caddy on first boot; open your address once DNS resolves and it answers. A tested run reached a working HTTPS login page about two and a half minutes after the Droplet was created.

Both providers bill hourly, so a short test costs cents. Keep the server running: synchronization, scheduled sending, and the exclusive lock on the data directory need one always-on instance. Add the provider's backups or snapshots; they must include both Dakia volumes and the Docker volumes that hold them.

### Proxmox

1. Use a Debian or Ubuntu VM with persistent local disk storage.
2. Install Docker Engine and Compose inside that guest, following the [Docker installation guide](https://docs.docker.com/engine/install/).
3. Follow the Compose and HTTPS steps above inside the VM. If using a proxy in another VM, account for the loopback binding described above.
4. Keep the guest running for background work. Test backup and restore before depending on it.

A dedicated appliance image or cloud-init installer is not supplied. Nested Docker in LXC is outside the tested path.

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

Run `sh scripts/upgrade.sh` from your installation directory to check published releases, back up, and upgrade. Use `--check` to review a release without changing the installation. The command requires Docker Compose and a shell, and briefly stops Dakia while making its backup.

The local owner also sees a dismissible release notice in the app. See [upgrading and recovery](upgrading.md) for existing-installation setup, preview releases, private backup storage, recovery, and manual NAS/Portainer upgrades.

Pulling the currently pinned image alone does not select a new version. Keep both persistent volumes and the credential key. Never start an older image against migrated data; recovery uses a matching restored snapshot in fresh storage.
