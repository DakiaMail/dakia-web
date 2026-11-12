# Security

This is an initial overview, not a security audit or a guarantee. A fuller threat model and hardening guide are planned.

## Login and ownership

New local installations require a private setup code from the host before creating the first owner account. Passwords are stored as salted Argon2id hashes. There is no public registration. The owner can create or revoke single-use invitations that expire after 24 hours. Invited people have separate logins and cannot manage invitations or access another user’s mail through the app. Invitation tokens are stored as hashes, and links use URL fragments to keep tokens out of server request URLs. Login attempts are rate limited per username with an instance-wide cap, and password hashing concurrency is bounded. Local password reset, user removal, and ownership transfer do not have management UIs yet; preserve access to your host and backups. See [using invitations](getting-started.md#invite-another-person) for the owner and recipient steps.

Existing OIDC installations retain their confidential-client, PKCE, nonce, and browser-bound state flow. OIDC and local identities are not automatically linked.

Sessions use random tokens, stored as hashes, in Secure, HttpOnly, SameSite cookies. API mutations require the configured origin and a session CSRF token. Local login forms require the configured origin. Use HTTPS and preserve the Origin header through your proxy.

## Credentials and mail

Mailbox credentials are encrypted using an operator-owned key stored separately from SQLite. This does not encrypt all message content at rest. Protect data volumes, host access, and backups; use disk encryption where appropriate. A compromised running host can access mail and decrypted credentials.

Google and Microsoft mailbox authorization is separate from Dakia login. OAuth state is single-use, short-lived and bound to the signed-in Dakia session, with PKCE. Refresh tokens are stored only inside encrypted mailbox credentials; Microsoft refresh-token rotation uses compare-and-swap so an old request cannot overwrite a reconnection. Operators own the application registration and secrets.

Account ownership is checked on the server. Received HTML is sanitized and isolated. Remote images follow user preferences and pass through the bounded image proxy. Mail-provider TLS verification must remain enabled.

## Containers and recovery

The supplied container runs without root privileges. Compose drops capabilities, uses a read-only root filesystem, and exposes its port on loopback. Do not give Dakia the Docker socket or privileged access.

Keep the encryption key through upgrades. A missing key on an existing data directory causes startup to fail. Backups must include the data and separately protected keys. Test restoration with disposable data before depending on it. Restore invalidates pending invitations as well as sessions.

## Reporting a vulnerability

Do not post secrets, private messages, or exploit details in public issues. Use [GitHub private vulnerability reporting](https://github.com/DakiaMail/dakia-web/security/advisories/new), which is enabled for this repository. A documented security contact and supported-version policy are being prepared.
