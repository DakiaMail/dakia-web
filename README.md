<p align="center"><img src="apps/web/public/dakia-icon-512.png" alt="Dakia logo" width="64"></p>
<h1 align="center">Dakia</h1>
<p align="center">Your mail, on your own server.</p>

Dakia is a free, open-source, self-hosted email client, not a mail server. Keep your existing email providers and bring your IMAP/SMTP mailboxes together on desktop and mobile.

![Dakia's unified inbox and open message, with fictional Work and Personal mailboxes](docs/images/inbox-desktop.webp)

- Bring multiple mailboxes into one inbox without moving your email to a new provider.
- Read conversations, organize with smart categories, and classify or translate mail on your own server.
- Use the same responsive app in your desktop browser or on your phone, with PWA installation available.

[Website](https://dakiamail.com) · [Features](docs/features.md) · [Setup guide](docs/getting-started.md)

## Get started

Dakia is under active development. Back up your installation and try a disposable mailbox before relying on it for important mail.

You need Docker Compose and an HTTPS address through your reverse proxy or Tailscale Serve.

```sh
git clone https://github.com/DakiaMail/dakia-web.git
cd dakia-web
cp .env.example .env
# Set DAKIA_ORIGIN to your HTTPS address, without a trailing slash.
docker compose up -d
```

Open your HTTPS address and choose **Sign in**. Read your private setup code from the host:

```sh
docker compose exec dakia cat /secrets/setup-token
```

Create your username and a password with at least 12 characters, then connect your first mailbox. Your Dakia password is separate from your email provider’s password.

For Gmail, choose **Continue with Google** when Google sign-in is configured for your installation. Each operator supplies their own Google OAuth client. Dakia keeps the refresh token encrypted on your server and renews access automatically. App passwords remain available.

Follow the [Gmail setup guide](docs/gmail-oauth.md) to register your client and configure its ID and secret. The pinned image includes Gmail sign-in. For development, use a separate Google Cloud project and Dakia data directory.

To add another person, open **Workspace → Invite someone → Create invitation** and share the link privately. They choose their own login and connect their own mailboxes. Links expire after 24 hours and work once.

[Follow the setup, invitation, and sign-in guide](docs/getting-started.md). Keep both persistent volumes: losing the credential key means losing access to stored credentials.

Compose pulls a verified preview image for Linux AMD64 or ARM64. It is pinned by digest so an upstream rebuild cannot silently change your installation. Platform-specific NAS and Proxmox walkthroughs are still being validated.

## Keep Dakia updated

Run `sh scripts/upgrade.sh` from your installation directory. It checks published releases, makes a backup, and verifies the upgraded server. Use `--check` to review a release first. The local owner also sees a quiet notice when a release is available. [Upgrade and recovery guide](docs/upgrading.md).

## A quick look

![Nine-second tour: desktop inbox, open message, mobile inbox, then the same message on mobile](docs/images/inbox-tour.gif)


## What you can do

- Invite other people to create separate logins and private mailbox connections.
- Connect Gmail with Google sign-in, or other IMAP/SMTP accounts, and use a unified inbox.
- Read conversations, search mail, and organize messages with smart categories.
- Compose rich-text messages with attachments, drafts, and scheduled sending.
- Archive, snooze, star, move, and clean up messages in bulk.
- Block unwanted senders across your mailboxes, with optional cleanup of older emails.
- Classify mail on your server and translate on your device or server.
- Use the responsive web app or install it as a PWA.

[Features and screenshots](docs/features.md)

## Documentation

- [Getting started](docs/getting-started.md): owner setup, joining by invitation, connecting mail, and signing in.
- [Gmail sign-in](docs/gmail-oauth.md): operator-owned Google credentials, connection, reconnection, and development testing.
- [Hosting and deployment](docs/hosting.md): Docker, Proxmox, Mac mini, Synology, TrueNAS, Unraid, and Portainer.
- [Security](docs/security.md): login, encryption, isolation, backups, and reporting vulnerabilities.
- [Privacy](docs/privacy.md): where mail is processed and which services Dakia contacts.
- [Operations](deploy/README.md): existing OIDC installations, configuration, and backup commands.

If Dakia looks useful, [give the repository a star](https://github.com/DakiaMail/dakia-web) to help others find it.

## Development

Use the Rust version in `rust-toolchain.toml`, Node.js 24, npm, Python 3, and OpenSSL development libraries.

```sh
npm ci --ignore-scripts
npm ci --prefix workers/translation --ignore-scripts
npm ci --prefix workers/push --ignore-scripts
python3 scripts/fetch-classifier.py
npm run build
npm test
cargo test --workspace --locked
```

To build your own container image, run `docker build -t dakia:local .` and set `DAKIA_IMAGE=dakia:local`.

The classifier is downloaded from a pinned revision and checked against its SHA-256 manifest. Translation models are downloaded and verified when needed.

Contributions are welcome through pull requests. Keep changes focused and include relevant checks. Never include mailbox data, credentials, or private captures in an issue or pull request.


## License

Dakia is licensed under [MPL 2.0](LICENSE). Dependencies and model artifacts retain their own licenses. See [third-party notices](THIRD_PARTY_NOTICES.md).

Recent mail is saved in a bounded, user-scoped browser cache for faster reopening
and folder switching. Cached views appear after a fresh session and account check
then refresh in the background. All messages intersecting the mail-list viewport
are prefetched serially without marking them read or loading external images.
Signing out or removing an account clears saved mail in this browser and other
updated tabs. See [persistent local caching](docs/persistent-local-cache.md) for
limits, authorization, retention and offline behavior.

## Try Dakia

Explore the app at [demo.dakiamail.com](https://demo.dakiamail.com). For the sandbox build, checks, and deployment, see [the demo guide](docs/demo.md).

Translation can run on the host or on devices, with an administrator-controlled host fallback option. Choose the policy in Workspace → Translation. [Language packs, administration and verification](docs/on-device-translation.md).
