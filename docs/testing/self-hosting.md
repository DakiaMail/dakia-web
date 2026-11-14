# Self-hosting setup exercise, 15 November 2026

This exercise followed the public setup and maintenance guides in a dedicated Ubuntu 24.04 ARM64 VM, with host file sharing disabled, a separate Docker daemon, and fresh Compose projects and volumes. It used fictional users and a disposable mail server. No existing deployment, real mailbox, or provider credentials were used.

## What changed after testing

| Observation | Change |
| --- | --- |
| The root installer created `/opt/dakia` with mode 700 and `.env` with mode 600, both owned by root. A normal user couldn't enter the directory to follow maintenance instructions. | Make a user-owned Compose directory the primary path; document the root shell needed by legacy installer installations. |
| The installer installed Docker by executing Docker's separate convenience script. | Link Docker's official package-repository instructions and keep the Dakia installer optional. The main app setup doesn't execute a host installer script. |
| The old Compose pin ran source `9bfb624262879582b1107b38a18e1276ae3e6961`, whose login page still required a setup code. | Pin the verified multi-platform image for source `2afaba9236efe190f921bb84abfd242dcaafc346`, and test its first-login flow. Retain instructions for older setup-code images. |
| Explicit Caddy `-f` commands omitted the automatically loaded override used by provider and OIDC examples. | Copy the Caddy example to `compose.override.yaml` and use ordinary Compose commands. Explain where additional service settings belong and validate with `config --quiet`. |
| Backup instructions described binary arguments without a complete Docker procedure. | Add commands for snapshot, keys, configuration, matching-image retention, separate restore, activation, and explicit job enablement. |
| The release helper had no published GitHub release to offer. | Explain that source image pins and tagged releases are separate, and provide a manual image-change path. |

The selected image is:

```text
ghcr.io/dakiamail/dakia-web@sha256:359c8bfa84ad67278e0715afd9565d06560c2fd4262af090c5d1d8d863f2a5e8
```

Its [published image workflow](https://github.com/DakiaMail/dakia-web/actions/runs/43644888999) passed native AMD64 and ARM64 packaging checks before this exercise. Local runtime checks below used ARM64. This workflow result belongs to the selected application image, not to the later documentation diff.

## Local results

- A real public repository clone and user-owned setup produced a mode-700 directory and mode-600 `.env`.
- The revised pin pulled successfully in the VM. `config --quiet`, `pull`, `up -d --wait`, and automatic Caddy override loading succeeded. A second clean project repeated first-owner setup and invitation joining with fresh volumes.
- HTTPS was verified with Caddy's disposable local CA. HTTP redirected to HTTPS. The API rejected anonymous access, and mutations rejected a mismatched Origin.
- The actual app container ran as `1000:1000`, with a read-only root filesystem, all capabilities dropped, and port 3025 bound only to loopback. No Docker socket was mounted.
- Owner creation needed no setup code. An invitation created a separate user; that user couldn't manage invitations or read the owner's connected mailbox, and their own account list stayed empty.
- A real TLS Dovecot IMAP service and local TLS SMTP sink exercised account connection, authentication, initial sync, reading, sending, and Sent filing. A hostname-mismatched certificate was rejected before mail access. The sent message reached the controlled fixture mailbox.
- The backup and offline-restore commands were extracted from the draft operations guide and executed. The restore preserved the credential key, owner, mailbox, and unsent draft. The backup contained sessions; the restored database cleared them and disabled background jobs.
- The documented restore override activated separate restored storage. The owner signed in again, the account and draft remained available, and the matching offline operator enabled jobs after reconciliation. Only one SMTP delivery occurred; restoration didn't resend it.
- The VM was restarted. Docker, Caddy, and Dakia returned automatically; HTTPS, owner login, the mailbox, and the draft remained available.
- The optional release helper's `--check` reported that no published release was available and left the healthy container unchanged.
- Offline packaged classification and translation-runtime smoke checks passed. The focused installer and upgrade test suites passed locally.

## Scope and reproduction

The repeatable app steps are in [hosting](../hosting.md), [getting started](../getting-started.md), [operations](../../deploy/README.md), and [upgrading](../upgrading.md). The exercise used Docker Engine 29.9.3 and Compose 5.7.1.

For local testing, `DAKIA_ORIGIN=https://localhost` selected Caddy's local CA. Clients explicitly trusted that disposable CA; certificate verification was not disabled. Distinct `COMPOSE_PROJECT_NAME` values isolated the rounds. The mail test alone added a read-only fixture CA and a hostname mapping to the VM's mail service. Those are test fixtures, not production setup requirements.

The baseline installer installed Docker first. Its slow image pull was interrupted and the identical old public image was imported from an existing cache to inspect its runtime. The revised image then completed an ordinary registry pull. The no-installer app setup was tested on that Docker-enabled VM; a second clean operating-system installation using Docker's package instructions wasn't performed.

Public DNS, public ACME issuance, provider firewall rules, Tailscale authentication, NAS interfaces, and live Gmail/Microsoft OAuth weren't exercised. The TLS mail fixture establishes protocol behavior for this setup, not universal provider acceptance. No cross-version upgrade of a populated owner mailbox or off-host disaster recovery is claimed. A separate restore was also tested outside the completed backup directory, as the final guide now recommends.

The VM and private fixture artifacts are retained for inspection. Keep authentication cookies, synthetic private keys, database snapshots, and raw transcripts outside Git.
