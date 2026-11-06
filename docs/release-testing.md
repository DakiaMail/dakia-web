# Release testing

Run `python3 scripts/release-check.py --profile full` for the full release checks.

The keyboard backend suite (`apps/web/playwright.keyboard-backend.config.ts`) exercises focus, editing, saved drafts, and durable mail actions through the real Rust API and SQLite, using disposable provider data. It is included in the full release runner. It does not send SMTP or use owner mail.
