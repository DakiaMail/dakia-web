# Documentation screenshots

Refreshed on 2026-11-16 from frontend revision [`33a4607`](https://github.com/DakiaMail/dakia-web/commit/33a4607935cbfa98762473de5a1d2c6488db7783), after PR #39 merged. All nine images show the running app in an isolated Google Chrome session, with fictional mail, names, addresses, and attachments.

## Inbox, sending, and mobile

The inbox, composer, and mobile scenes use the existing demo mailbox in `apps/web/src/demo/seed.ts` and the same frontend components as the server build. No real mailbox connection or email delivery is involved.

```sh
DAKIA_BUILD_ID=33a4607935cbfa98762473de5a1d2c6488db7783 npm run build:demo
npm exec --workspace @dakia/web -- vite preview --host 127.0.0.1 --outDir dist-demo --port 5188 --strictPort
```

Open `http://127.0.0.1:5188/?reset=1` in an isolated Chrome profile. Desktop viewport: 1440 × 960. Mobile viewport: 390 × 844. Device scale: 1. Locale: English (en-US). Timezone: Europe/Tallinn. The demo clock is fixed at 2026-11-16 12:00 EET before seeding the mailbox.

| Image | Scene |
| --- | --- |
| `personal.webp` | Personal inbox across both accounts, with Maya’s launch conversation and attachment open |
| `automated.webp` | Automated inbox, type filters, extracted details, and an open receipt |
| `transactions.webp` | Transactions selected, with matching receipts and a receipt attachment |
| `attention.webp` | Needs attention view with a verification code open and a cancelled flight in the list |
| `compose.webp` | New message to Maya with Send later and Pick date & time open, cropped to the composer and picker |
| `inbox-mobile.webp` | Personal inbox with the search toolbar at the top and attention summary collapsed |
| `reader-mobile.webp` | Maya’s conversation in dark theme, with an attachment and reply controls |

Mobile images are responsive Chrome captures. The reader scene starts in a fresh demo session with dark theme selected before opening the message. Email-authored rendering colors and normal scrolling are retained. Extra keyboard hints are left off. The captures contain no unread-status notice, image-failure notice, or end-of-list message.

## Real on-device translation

The translation pair uses the production frontend build and the existing disposable Rust fixture `language::browser_tests::browser_device_translation_fixture`, listening on loopback port 4346. The fixture session values are defined in `apps/web/browser/device-translation-backend.spec.ts` and authorize only fictional fixture data.

```sh
DAKIA_BUILD_ID=33a4607935cbfa98762473de5a1d2c6488db7783 npm run build
DAKIA_WORKERS="$PWD/workers" \
DAKIA_MODEL_CACHE=/private/tmp/dakia-production-translation-models \
  cargo test -p dakia-server --lib browser_device_translation_fixture -- --ignored --nocapture
```

A loopback proxy on port 5190 serves the freshly built frontend and forwards fixture API calls to port 4346, preserving its origin for CSRF checks. The existing compiled fixture was reused; the frontend and browser translation engine come from revision `33a4607`.

The screenshot scene gives the fictional message the subject “Neljapäevane kohtumine”, sender Liis Tamm (`liis@atelier.example`), and recipient Alex Morgan (`alex@northline.example`). Only fictional display metadata, mailbox lists, and account counts are controlled for the scene. The original message body, including its English footer, is unchanged.

The translation policy is set to `device`. The browser downloads the Estonian files from the fixture’s authenticated model endpoint and runs the real WASM engine. All three cached files were checked against the pinned byte sizes and SHA-256 hashes in `workers/translation/models.json`. Cold external model downloads were not repeated.

`translation-original.webp` shows the original before translation. `translation.webp` shows the browser-generated English result with Show original available. The rendered interface and translated output were not repainted.

## Export

Google Chrome version: 157.0.8103.51. Source captures are PNG. Exports use Pillow WebP quality 90 and method 6, without source metadata. Aspect ratios are retained; only the composer image is cropped. All nine exported images were visually inspected.

| File | Pixels | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `attention.webp` | 1440 × 960 | 36808 | `68302a86f3e5dc3d44d3ab5eb2a4ef59298aaa8e549cbfb47621f4df5b976e80` |
| `automated.webp` | 1440 × 960 | 81174 | `ca86646f0b8872e73cb3d51c557b760fe1efd83e02c9360761c2155b985c49b0` |
| `compose.webp` | 784 × 550 | 18100 | `34c636ee14ab9a1efe0dc2735dfcd8bf953ce1016d56742690952a6fcb25d389` |
| `inbox-mobile.webp` | 390 × 844 | 45720 | `f30264abf48a10ba2cec34f0300b73e68e74a8ce8fe35a374a82fffffc9fa460` |
| `personal.webp` | 1440 × 960 | 103140 | `42ad40756da326c1af074b7aadd39253b19e4f94ee0e79823e0c0df5014b7fe9` |
| `reader-mobile.webp` | 390 × 844 | 22596 | `58bdd7f1fe1e5504bd10b62c03e9623c59f2414f088ca1f5dbab25ea582a391c` |
| `transactions.webp` | 1440 × 960 | 52362 | `eb4d73fdfb8aa7e2581dde755b6d04433bff9add7becbde0967e38e77830130b` |
| `translation-original.webp` | 1440 × 960 | 55882 | `87b7cb6b30a4e5a91ef975a37efb5639e761391f14ffe4d0635bc4dfa6a5e992` |
| `translation.webp` | 1440 × 960 | 55658 | `08c1d5d57ebdabc9a42e9afadd8cc245b799edd2beadad1c1b8feb0e7ef5f179` |
