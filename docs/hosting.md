# Hosting Dakia

This guide covers the initial self-hosting package. Platform-specific walkthroughs and tested resource requirements are being prepared. Do not treat an untested platform as supported.

## Docker Compose

Follow the root README. Set `DAKIA_ORIGIN` to the exact HTTPS address users open. Compose binds port 3025 to host loopback. Put an HTTPS reverse proxy on the same host in front of it, or use Tailscale Serve. A proxy in a separate container must share a private Docker network with Dakia; its localhost does not refer to the host or Dakia container.

The application requires HTTPS and uses Secure cookies. Plain HTTP on a LAN IP is not supported. Do not expose port 3025 directly to the internet. The application needs outbound HTTPS, IMAP, and SMTP connectivity, not inbound SMTP ports or MX changes.

The `data` volume contains SQLite, messages, attachments, and model caches. The `secrets` volume contains the credential key and initial claim code. Both must survive container recreation. Never use `docker compose down -v` on an installation you want to keep. Back up data and secrets together, with restricted access and encrypted off-host copies. Keep live SQLite off SMB/NFS shares.

## Proxmox

Create a Debian or Ubuntu VM, install Docker Engine and Compose, and use the same package. Do not install Dakia on the hypervisor. A ready-made cloud-init flow is planned. LXC nesting is not part of the initial tested path.

## Mac mini

Install a Docker runtime with Compose, then follow the Docker instructions. Apple Silicon needs the ARM64 image; Intel needs AMD64. Test restart after reboot, configure the runtime to start automatically, and keep the machine awake for background syncing and scheduled mail. Native macOS packaging is not included in this phase.

## Synology

On a model supporting Container Manager, create a Project from the Compose file. Configure storage permissions for UID 1000, your HTTPS reverse proxy, and the external origin. Check the processor architecture before installing. A screenshot walkthrough is planned.

## TrueNAS

Use Apps > Install via YAML with the Compose definition. Adjust storage to dedicated local datasets and permissions for UID 1000. Keep secrets private. A validated catalog entry is planned.

## Unraid

Use the container image with persistent appdata and a separate private secrets directory. Map container port 3025 through your HTTPS proxy. An application template and platform testing are planned.

## Portainer

Deploy the Compose definition as a stack. Use the pinned registry image from the Compose file and set the origin explicitly. A ready-made template is planned.

## Private access

Tailscale Serve can provide an HTTPS address inside your tailnet without router forwarding. Set that exact address as the origin. Your existing Caddy, Traefik, or Nginx Proxy Manager installation is another option. Detailed proxy examples are planned.

## Existing OIDC installations

Keep the existing issuer, client ID, secret file, origin, credential key, and data mounts. A fully configured OIDC installation continues using its original login flow and user identities. Local login is enabled only when OIDC is absent. Removing OIDC settings from an installation with existing OIDC users fails startup rather than allowing someone to claim their mail.

## Images and updates

The root Dockerfile builds both architectures. The image workflow runs only on manual dispatch or version tags, never on main pushes, and does not deploy to servers. The published preview image has passed native builds and packaged startup, worker, health, and restart checks on both architectures. This does not establish compatibility with every NAS or container runtime. The Compose default pins that image by digest.

Keep the previous image and a matching stopped backup before upgrades. Database migrations can require restoring a matching backup for rollback. See the operations guide for backup and restore commands. Automated backups and guided updates are future work.
