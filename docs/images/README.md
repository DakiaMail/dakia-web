# Documentation screenshots

Captured on 2026-11-15 from source revision `c63030c`, using the running Dakia frontend in an isolated Google Chrome session. All mail, names, addresses, and attachments are fictional. No real mailbox, personal browser profile, provider connection, or email delivery was used.

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
| `reader-mobile.webp` | 390 × 844 | 22564 | `9a283eb0b6c28981288d7ad2260997baefa3e3340d09d6764ede98d82bc151a0` |

Mobile images are responsive browser captures, not native phone screenshots. The demo read-state notice was dismissed with its normal control before capturing the mobile reader. Normal scroll boundaries, toolbar behavior, and the app's email-rendering colors are retained.

## Real on-device translation

The translation pair uses the production frontend build and the existing disposable Rust translation fixture:

```sh
npm run build
DAKIA_WORKERS="$PWD/workers" \
DAKIA_MODEL_CACHE=/private/tmp/dakia-production-translation-models \
  cargo test -p dakia-server --lib browser_device_translation_fixture -- --ignored --nocapture
```

The fixture listens on loopback port 4346. Its test session and CSRF values are defined in `apps/web/browser/device-translation-backend.spec.ts`; they authorize only this disposable fixture. Set the translation policy to `device`, then open its `estonian-english-footer` message.

For the screenshot scene, intercepted API responses give the fictional message the subject “Neljapäevane kohtumine”, sender Liis Tamm (`liis@atelier.example`), and recipient Alex Morgan (`alex@northline.example`). Search and conversation responses contain that message with complete coverage. Account addresses are fictional. The original HTML body is unchanged. The full desktop viewport includes the new search toolbar. No application code, translated output, or rendered interface was repainted.

The capture uses the previously verified Estonian weights in the isolated model cache, checked against the pinned byte sizes and SHA-256 digests in `workers/translation/models.json`. The authenticated model endpoint delivers them to Chrome, and the real browser WASM engine generates the English text. This refresh does not retest cold model downloads.

For this refresh, a loopback HTTP proxy on port 5190 serves the newly built `apps/web/dist` and forwards fixture API requests to port 4346, preserving the fixture origin for its CSRF checks. This allows the existing compiled test fixture to be reused without rebuilding the Rust server. The screenshot route interception described above changes only fictional scene data.

`translation-original.webp` shows the original before translation. `translation.webp` shows the browser-generated English result with Show original available. Both are full 1440 × 960 desktop captures. This is evidence of local inference, not cold-download reliability, universal device support, or live-provider acceptance.

## Export

Google Chrome reported version 157.0.8103.51. Source captures were PNG. Exports use Pillow WebP quality 90, method 6, without source metadata. Images retain their aspect ratio; only the composer is cropped. The superseded inbox tour and older Workspace/mobile captures have been removed.

| File | Pixels | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `attention.webp` | 1440 × 960 | 29524 | `1d724bbd4a1753bef4fb89f5d1aa1af6b80889655ef7dbfbd06e7636a0258020` |
| `automated.webp` | 1440 × 960 | 79088 | `f22cbe69facd83aa82e706d9e0a5fbebac277b2757ade646a3b2d78e4862acde` |
| `compose.webp` | 800 × 580 | 19902 | `297b2b04471a76c9e6823b563518c24d2bc6eefbc6a1ab99dd2462282bd34a3f` |
| `inbox-mobile.webp` | 390 × 844 | 42452 | `79dbe72e1184cbe1d690b3644c7fc211293634e17f12733cf4afd3445b03332b` |
| `personal.webp` | 1440 × 960 | 103696 | `fd43279417b2f085a94c8e96ea34b7b64c7764df19a395752f93a017ee7fba8f` |
| `reader-mobile.webp` | 390 × 844 | 22564 | `9a283eb0b6c28981288d7ad2260997baefa3e3340d09d6764ede98d82bc151a0` |
| `transactions.webp` | 1440 × 960 | 54008 | `cbb701dfd85856775987e4e4390e675b8a345bd934a6f54006b33a03c9b7f1c1` |
| `translation.webp` | 1440 × 960 | 57798 | `c8c04fe0b12d6381966d7c3b2c4159ca3ae3d23451cbf6fc220558b1c6e36c47` |
| `translation-original.webp` | 1440 × 960 | 58016 | `c78a3ecdf277f5da89c14db4f47bb6c70f7b290ea3e7f0fdfb3d46fa172fcf83` |
