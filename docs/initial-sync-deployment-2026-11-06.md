# Initial sync deployment, 6 November 2026

The full-body initial-sync improvements from [PR #72](https://github.com/DakiaMail/dakia-web-private-history/pull/72) were merged as `af5f8019c90e63f31cca646ef6e00f68a3e34839`. The packaged backend uses source revision `92ceb20cf8710c6f89adc44fe098f949bf72eca7`, including the formatting-only follow-up to the measured implementation.

The [benchmark report](initial-mailbox-sync-benchmark.md) records 50,000 downloaded and indexed bodies in 316.82 seconds against pl_mail's 401.73 seconds, with resource accounting and platform limitations. This deployment is Linux amd64. The benchmark was macOS arm64 versus Linux arm64 Docker; no new production performance ratio is claimed.

## Build and release checks

- Uploaded a Git archive and verified SHA-256 for all 534 committed source files against the final source revision.
- Built with `cargo build --release --locked -p dakia-server` in the existing Rust 1.98 / Ubuntu 24.04 Linux amd64 builder, using isolated source and build-cache mounts.
- Replaced only `dakia-server`, `dakia-admin` and `dakia-healthcheck` in an overlay of the current production image. The frontend remains revision `050bed79947e`.
- Passed formatting and strict release Clippy checks for all targets of the three changed backend crates.
- Passed server, provider and storage release regressions. The server suite has 126 passing tests and 13 intentionally ignored fixtures. The provider/storage command includes integration and documentation tests.
- Passed nine Google Chrome scenarios against the real Rust release backend and disposable mail: body-only Unicode search and suggestions at mobile/desktop widths, exact attachment bytes, cached/provider search continuation, archive, trash, spam, unread persistence and durable sender cleanup. No API response mocks were used.
- In the final image, passed dynamic-library checks for all three binaries, the translation worker smoke test, and network-disabled server startup against temporary data. Health and the app returned 200; anonymous account access returned 401.

## Activation and recovery

Activation holds the deployment lock and checks the expected previous image. It saves private incident logs, checks for in-flight owner mutations/submissions, stops the app and copies its persistent data. Every copied file is hash-verified; SQLite quick checks and message/operation/draft/submission ID sets are verified before restart.

The first activation automatically rolled back because its environment comparison treated list ordering as meaningful. The previous app returned healthy. The guard was changed to compare all environment keys and values, retaining the mount, security-setting and identity-provider checks. A fresh backup is used for the retry. Environment values must still match exactly.

The retry succeeded. The deployed image is `sha256:b7ba3508cb9cfb1f222e20bdfd2a75fe0c92ff528eb96451ed3a29384ca96c33`, tagged `dakia-web:initial-sync-92ceb20cf871`. The server binary SHA-256 is `7cf6f06340b0c38ecc85e270c9b6778bb00e03b32a318662fe1eb20c472e0012`.

All 11,419 backup files were verified. Message, operation, draft and submission ID sets were preserved, schema versions stayed unchanged, and environment values, mounts and container security settings matched. Web, worker, model and license assets matched byte-for-byte. The identity-provider container was unchanged.

## Live app verification

- Docker health is healthy with zero restarts. HTTPS health, app, service worker and frontend version endpoints return 200. Anonymous account access returns 401.
- Using the user's signed-in Chrome session, both accounts returned folders (20 and 22) and 50-row mailbox pages with HTTP 200. Indexed search also returned 200.
- A keyboard-submitted synthetic search rendered an empty result without alerts. Submitting a cleared search restored the 21-row mailbox view. No owner message was opened or changed for these checks.
- Aggregate index coverage stayed at 45,744 headers, 45,534 ready bodies, zero pending and 210 existing coverage gaps. No new gap appeared during this verification window.
- The post-deployment log window contained zero `ERROR` lines and zero panic lines. Private runtime logs stay on the host.

These are startup and functional checks, not a long-duration soak. The [sanitized deployment verification](../benchmarks/initial-sync/evidence/2026-11-06/deployment-verification.json) records binary hashes, backup verification, preservation guards and aggregate authenticated results.

## Evidence

The [benchmark evidence directory](../benchmarks/initial-sync/evidence/2026-11-06/) includes compressed final release-test, strict Clippy and Chrome logs. Production mailbox contents, credentials, session cookies, private incident logs and database backups are excluded from Git.
