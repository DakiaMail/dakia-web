# Documentation screenshots

Captured on 2026-11-14 from source revision `1c104a7`, using the running Dakia frontend in an isolated Google Chrome session. All mail, names, addresses, and attachments are fictional. No real mailbox, personal browser profile, provider connection, or email delivery was used.

## Inbox, sending, and mobile

The normal app runs with the existing demo mailbox from `apps/web/src/demo/seed.ts`. The demo uses the same frontend components as the server build; actions are handled by its browser-local transport. These captures demonstrate the interface, not provider sync or delivery.

```sh
npm ci --ignore-scripts
npm run dev --workspace @dakia/web -- --mode demo --port 5188
```

Open `http://127.0.0.1:5188/?reset=1` in an isolated Chrome profile. Desktop viewport: 1440 × 960. Mobile viewport: 390 × 844. Device scale: 1. Locale: English. Timezone: Europe/Tallinn. The demo dates are relative to its initial creation time; the clock wasn't frozen.

| Image | Scene |
| --- | --- |
| `personal.webp` | Personal tab across both accounts, Maya's launch conversation and attachment open |
| `automated.webp` | All automated mail, copyable details, type filters, and a receipt open |
| `transactions.webp` | Transactions selected, with matching receipts and an attachment |
| `attention.webp` | Needs attention sidebar view with a verification code and cancelled flight |
| `compose.webp` | New message to Maya, with Send later → Pick date & time open; cropped to the composer and picker |
| `inbox-mobile.webp` | Personal inbox in the narrow browser layout |
| `reader-mobile.webp` | Maya's conversation in the narrow layout, with dark theme enabled |

Mobile images are responsive browser captures, not native phone screenshots. Normal scroll boundaries, toolbar behavior, and the app's email-rendering colors are retained.

## Real on-device translation

The translation pair uses the production frontend build and the existing disposable Rust translation fixture:

```sh
npm run build
DAKIA_WORKERS="$PWD/workers" \
DAKIA_MODEL_CACHE=/private/tmp/dakia-production-translation-models \
  cargo test -p dakia-server --lib browser_device_translation_fixture -- --ignored --nocapture
```

The fixture listens on loopback port 4346. Its test session and CSRF values are defined in `apps/web/browser/device-translation-backend.spec.ts`; they authorize only this disposable fixture. Set the translation policy to `device`, then open its `estonian-english-footer` message.

For the screenshot scene, intercepted API responses give the fictional message the subject “Neljapäevane kohtumine”, sender Liis Tamm (`liis@atelier.example`), and recipient Alex Morgan (`alex@northline.example`). Search returns that message and its conversation snapshot contains that message. The original HTML body is unchanged. The capture is cropped to the conversation, excluding unrelated fixture transport errors. No application code, translated output, or rendered interface was repainted.

The initial cold model request timed out. Public Estonian weights were then downloaded separately, checked against every pinned byte size and SHA-256 digest in `workers/translation/models.json`, and placed in the fixture's isolated model cache. The normal authenticated model endpoint delivered them to Chrome successfully. The real browser WASM engine translated the message, with no host translation POST. Browser Cache Storage contained the engine and three verified model artifacts after translation.

`translation-original.webp` shows the original before translation. `translation.webp` shows the browser-generated English result with Show original available. Both are conversation-element captures cropped to 582 × 545. This is evidence of local inference and saved artifacts, not cold-download reliability, universal device support, or live-provider acceptance.

## Export

Google Chrome reported version 157 via its user agent. Source captures were PNG. Exports use Pillow WebP quality 90, method 6, without source metadata. Images retain their aspect ratio; only the composer and translation pair are cropped. The superseded inbox tour and older Workspace/mobile captures have been removed.

| File | Pixels | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `attention.webp` | 1440 × 960 | 39730 | `5a7d8bc988e8d22f7ccb719e2793a61e213a2d7661cad5f47c3386490ec115ac` |
| `automated.webp` | 1440 × 960 | 78194 | `b3a8c23504341c7bb7531bc436172c6f350ae0d8d55677d7434df4150b9ef202` |
| `compose.webp` | 800 × 580 | 17650 | `23037b9c80cb165a7b282ae9362c4b87b72630ca3c3029ea1291844b64ffc2d1` |
| `inbox-mobile.webp` | 390 × 844 | 49524 | `13093cb011fb44b37feb299677203c662bf57d898f793ec5a74b977f5356ad8e` |
| `personal.webp` | 1440 × 960 | 103146 | `0ad0b88ef0f74897966f1bed597ee5e794c7819949d48f2f9cb6ba41eec2e284` |
| `reader-mobile.webp` | 390 × 844 | 22334 | `e158ac88dbba80b807fdfdc58456660a899bb7c724f12e8e18115d85f2435e18` |
| `transactions.webp` | 1440 × 960 | 53558 | `f08ea173f928d3092404c31e53da7cdced49c2864b9e644f3345197fbce5f47d` |
| `translation-original.webp` | 582 × 545 | 33526 | `f7746e8192378fff484644ed8fa32b371c39f759c0ba651b31fbb973e8d28656` |
| `translation.webp` | 582 × 545 | 32840 | `d2529f837169669ec2491d33ea1f85df69628fe0123275133d35bc2accd3a072` |
