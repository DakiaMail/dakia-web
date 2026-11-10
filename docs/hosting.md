# Hosting Dakia

Use the published container image with Docker Compose. It includes the web app, server, classifier, and workers; a source build and a separate database service are not required. The image supports Linux AMD64 and ARM64.

Start with [owner setup and everyday use](getting-started.md) after installation. For configuration, backups, and existing OIDC installations, see [operations](../deploy/README.md).

## Docker Compose

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
| `/secrets` | Credential encryption key and the initial setup code |

The supplied Compose file creates named volumes for both. They must survive container recreation. Never run `docker compose down -v` on an installation you want to keep. Back up data and secrets with restricted access and encrypted off-host copies. Losing the key prevents decryption of stored mailbox credentials.

The container runs as UID/GID 1000. If replacing named volumes with host directories or NAS datasets, create dedicated directories writable by that identity and keep the secrets directory private. Keep live SQLite on local storage; do not put its database files on an SMB/NFS mount from another machine.

Allow outbound HTTPS, IMAP, and SMTP connections. Dakia does not require inbound SMTP ports or MX changes. Provider connectivity and storage capacity matter as mailboxes grow; minimum hardware requirements are not yet established.

## Platform paths

The Compose package and native AMD64/ARM64 images have been checked, and a Docker VM under Proxmox has been exercised with separate users and disposable TLS mailboxes. The NAS paths below describe manual configuration; platform-specific acceptance and catalog packages are not yet complete.

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
