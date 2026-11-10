# Release testing

Run `python3 scripts/release-check.py --profile full` for the full release checks.

The keyboard backend suite (`apps/web/playwright.keyboard-backend.config.ts`) exercises focus, editing, saved drafts, and durable mail actions through the real Rust API and SQLite, using disposable provider data. It is included in the full release runner. It does not send SMTP or use owner mail.

## Self-hosted upgrades

Run the portable release-selection and host-orchestration tests without Docker:

```sh
node --test scripts/tests/upgrade*.test.mjs
```

The image workflow runs these before its AMD64/ARM64 builds. Packaged OAuth smoke checks also verify release endpoint authorization, opt-out, and the compiled release version. Version tags publish a release manifest and paired upgrade scripts only after both runtime checks pass.

For real upgrade/recovery acceptance, pre-pull two immutable published images with a control-database migration between them, then run:

```sh
node scripts/tests/upgrade-docker.mjs OLD_IMAGE_DIGEST_REFERENCE NEW_IMAGE_DIGEST_REFERENCE
```

This creates a fresh project with mail networking disabled, seeds a disposable owner and encrypted mailbox credential, and exercises the real upgrade script, database migration, persistent pin, backup, and separate restore. Release lookup is replaced by a fixed test manifest and pulling by a local-image check; production discovery is unchanged. The test removes only its own Compose volumes and retains its private temporary backup directory for inspection. It does not publish a GitHub release or validate both CPU architectures.

Run `apps/web/browser/releases.spec.ts` in Chrome for owner-only notice visibility, keyboard focus, dismissal persistence, mobile navigation, and light/dark layouts. For a manual review with fictional mail, build the web app and run `node scripts/tests/release-ui.mjs`; open `http://127.0.0.1:4184` in Chrome. This fixture has no real accounts or mail-provider access.
