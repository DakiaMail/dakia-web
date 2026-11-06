<p align="center"><img src="apps/web/public/dakia-icon-512.png" alt="Dakia logo" width="104"></p>
<h1 align="center">Dakia</h1>
<p align="center">Your mail, on your own server.</p>

Dakia is a self-hosted email app that connects to your existing mailboxes. Read, search, organize, and send mail from one place, on desktop or mobile.

Run it on a home server, a Mac mini, or a NAS. Your existing email provider continues to receive and deliver your mail.

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

To add another person, open **Workspace → Invite someone → Create invitation** and share the link privately. They choose their own login and connect their own mailboxes. Links expire after 24 hours and work once.

[Follow the setup, invitation, and sign-in guide](docs/getting-started.md). Keep both persistent volumes: losing the credential key means losing access to stored credentials.

Compose pulls a verified preview image for Linux AMD64 or ARM64. It is pinned by digest so an upstream rebuild cannot silently change your installation. Platform-specific NAS and Proxmox walkthroughs are still being validated.

## What you can do

- Invite other people to create separate logins and private mailbox connections.
- Connect multiple IMAP/SMTP accounts and use a unified inbox.
- Read conversations, search mail, and organize messages with smart categories.
- Compose rich-text messages with attachments, drafts, and scheduled sending.
- Archive, snooze, star, move, and clean up messages in bulk.
- Classify and translate mail on your own server.
- Use the responsive web app or install it as a PWA.

[Features and screenshots](docs/features.md)

## Documentation

- [Getting started](docs/getting-started.md): owner setup, joining by invitation, connecting mail, and signing in.
- [Hosting and deployment](docs/hosting.md): Docker, Proxmox, Mac mini, Synology, TrueNAS, Unraid, and Portainer.
- [Security](docs/security.md): login, encryption, isolation, backups, and reporting vulnerabilities.
- [Privacy](docs/privacy.md): where mail is processed and which services Dakia contacts.
- [Operations](deploy/README.md): existing OIDC installations, configuration, and backup commands.

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

## Keyboard controls

Use Tab and Shift+Tab to move between controls and composer fields, and Enter or Space to activate buttons. Mail shortcuts work outside text inputs, editors, dialogs, popovers and text selections. Browser/OS chords and IME input keep their normal behavior.

Use J/K for next/previous email, C to compose, / to search, R to reply, Shift+R to reply all, F to forward, E to archive, Delete to move to trash, U to mark unread, Shift+U to mark read, S to toggle stars, X to toggle selection, M to move, B to snooze, and ! to mark spam. Actions apply to checked messages first, then the focused row or open email. Reply and forward load the target email’s content when needed. Navigation follows visible mail order and stops at the first/last loaded email. Press ? for the full shortcut reference. Workspace settings can disable mail shortcuts or enable extra hints beside buttons and in the inbox footer. Preferences are saved per signed-in user on this browser.

Within the Inbox view tablist, unmodified Left/Right and Home/End select tabs. On message checkboxes, Space toggles selection and Shift+Space selects a range; Shift+Enter or Shift+Space on a message button extends the range without opening it. Recipient suggestions support unmodified Up/Down, Enter and Escape. Escape outside editors, text selections and dialogs clears the mail selection/reader; dialogs and the refresh tooltip handle their own Escape. OS modifier chords and IME composition are not consumed by these application handlers. The rich-text editor retains its own standard editing behavior.


## License

Dakia is licensed under [MPL 2.0](LICENSE). Dependencies and model artifacts retain their own licenses. See [third-party notices](THIRD_PARTY_NOTICES.md).
