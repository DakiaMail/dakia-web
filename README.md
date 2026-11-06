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

Create your username and password, then connect your first mailbox. Setup closes after the owner account is created. Keep both persistent volumes: losing the credential key means losing access to stored credentials.

Compose pulls a verified preview image for Linux AMD64 or ARM64. It is pinned by digest so an upstream rebuild cannot silently change your installation. Platform-specific NAS and Proxmox walkthroughs are still being validated.

## What you can do

- Connect multiple IMAP/SMTP accounts and use a unified inbox.
- Read conversations, search mail, and organize messages with smart categories.
- Compose rich-text messages with attachments, drafts, and scheduled sending.
- Archive, snooze, star, move, and clean up messages in bulk.
- Classify and translate mail on your own server.
- Use the responsive web app or install it as a PWA.

[Features and screenshots](docs/features.md)

## Documentation

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

## License

Dakia is licensed under [MPL 2.0](LICENSE). Dependencies and model artifacts retain their own licenses. See [third-party notices](THIRD_PARTY_NOTICES.md).
