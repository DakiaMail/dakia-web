# PikaPods catalog request

Dakia is a self-hosted email client, not an email server. PikaPods controls its app catalog; this document is a handoff for a catalog request, not a deploy button or a claim that Dakia is available there. Suggest the app through [PikaPods feedback](https://feedback.pikapods.com/) or contact PikaPods as described in its [app criteria](https://docs.pikapods.com/faq/apps).

- Project: https://github.com/DakiaMail/dakia-web
- License: MPL 2.0
- Image: `ghcr.io/dakiamail/dakia-web` (use the digest pinned in [`compose.yaml`](../compose.yaml) for the current preview). Linux AMD64 and ARM64.
- HTTP container port: `3025`. PikaPods terminates HTTPS at its proxy. Preserve the browser's `Origin` header.
- Required environment: `DAKIA_ORIGIN` is the exact external HTTPS origin, without a trailing slash or path. It must be updated if the pod domain changes. Optional Gmail OAuth settings are documented in [operations](README.md) and the [Gmail guide](../docs/gmail-oauth.md).
- Persistent writable storage: `/data` and `/secrets`, writable by UID/GID 1000. Keep both across restarts and upgrades. `/secrets` contains the credential encryption key and must remain private. Losing the key makes stored mailbox credentials unreadable.
- Outbound access: HTTPS, IMAP and SMTP submission. PikaPods [blocks port 25](https://docs.pikapods.com/faq/technical#sending-emails); mail providers must support a submission port such as 465 or 587. No inbound mail ports or separate database are needed.
- Resources: minimum CPU, memory and storage requirements have not been established; size and test with representative mailboxes.

## Before publishing

Coordinate with PikaPods to configure the image, persistent mounts and per-pod origin. No host shell is needed: the [first login](../docs/getting-started.md#create-the-first-login) is created in the browser and becomes the owner, so a new pod must not be reachable by anyone else until its owner has signed up. Do not put the credential key in logs or a public environment variable.

Test a new pod end to end: owner setup, mailbox connection and sending, restart with both mounts intact, domain changes, upgrades, and backup and restore of data **and** the credential key. PikaPods [manages app updates](https://docs.pikapods.com/faq/apps#updating-apps); the Compose upgrade script is not a pod upgrade procedure. Verify backup and recovery with PikaPods before depending on the service.
