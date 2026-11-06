# Mail provider

Account-scoped IMAP/SMTP adapters. Source includes scripted TLS fixtures and a disposable Dovecot runner, not live account credentials or mailbox data.

## Integration boundaries

`provider.rs` defines `MailProvider`, `SubmissionGate` and `AccountProvider`; provider DTOs are in `types.rs`. The server resolves storage's `AuthorizedAccount`, checks provider/account identity and retains a provider instance per account. Ownership, operation revisions and idempotency belong in storage/server. Credentials and filesystem paths do not cross the public API.

`probe` authenticates both protocols but does not submit mail. Folder discovery covers selectable folders including unsubscribed folders. Missing or ambiguous Archive/Trash mappings require an explicit mapping rather than a guessed destructive destination.

Scans use bounded metadata PEEK, account/folder/query/UIDVALIDITY-scoped continuations and full paged reconciliation to refresh flags and discover removals. A failed/incomplete scan must not prune memberships. Dedicated IDLE is a rescan hint with polling fallback, not a claim of CONDSTORE/QRESYNC support. Gmail identity merges use account plus X-GM-MSGID and preserve labels and X-GM-THRID. Generic IMAP identities remain scoped by account/folder/UIDVALIDITY/UID; RFC Message-ID alone is not a deduplication key.

Seen/star are desired-state STORE operations. Gmail archive removes Inbox membership. Generic move uses MOVE or COPY plus targeted UID EXPUNGE with UIDPLUS. Unsupported cleanup fails before modification where possible; partial/uncertain operations require reconciliation instead of duplicate COPY retries. There is no global EXPUNGE/CLOSE or permanent-delete API.

## Sending and Sent

MIME assembly uses the configured primary identity, validates recipients/headers, preserves reply headers and omits Bcc headers while retaining envelope recipients. Persist the exact immutable MIME bytes and identity before dispatch. Every RCPT must succeed before DATA. `SubmissionGate` persists the transition into submitting; lost acknowledgements/interruption become `delivery_unknown`, never an automatic resend.

Sent reconciliation runs only after persisted SMTP acceptance. Gmail SMTP is provider-managed. Generic providers search existing Sent candidates by the immutable Message-ID before APPEND, preserving explicit mappings and avoiding duplicate filing after uncertain acknowledgements. Missing or failed inspection fails safely. Filing retries must not resubmit SMTP. Counts and conventional-name ranking aid automatic Sent selection but are not a universal guarantee of provider intent.

The async-imap adapter checks decoded responses and matching tagged OK for commands whose streamed helpers can otherwise discard tagged failures. Tests cover rejected STORE/LIST, disconnected SEARCH and partial move cleanup.

## Checks

From the repository root:

```sh
cargo test --locked -p dakia-mail-provider
python3 crates/mail-provider/fixtures/run_dovecot.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

See [disposable fixture instructions](fixtures/README.md). Fixture tests are not a substitute for acceptance with a particular provider. Never point them at owner accounts.

## Source notice

SMTP stage handling adapts DakiaMail/dakia-desktop `crates/dakia-core/src/mail.rs`, licensed MPL-2.0. This crate is MPL-2.0. Retain attribution and source availability on distribution. Dependencies retain their individual licenses; see the root license and notices.
