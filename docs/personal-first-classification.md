# Personal-first classification

Dakia Web keeps three independent decisions on each message: Personal or Automated, overlapping detected types, and Needs attention. Personal is the conservative fallback while work is pending, fails, or has insufficient evidence. Reading never changes these decisions.

The inbox opens on Personal and remembers the last tab for that user. Automated has one optional filter at a time. Transactions combines receipts and invoices; the remaining filters select individual detected types. All and search retain their normal scope. Tabs keep separate scroll positions and selections. Needs attention is available above the inbox tabs and as a separate sidebar view. It covers inbox messages received in the last 30 days across the selected accounts; the same messages remain in their tabs. Snooze hides attention temporarily. Archive, trash and spam remove it from inbox attention.

## Rules file

Classification patterns live in [`crates/inference/rules/classification.json`](../crates/inference/rules/classification.json). Edit this file to change sender patterns, detected-type phrases, service notices, attention phrases, extracted values, status text, link labels, delivery-service headers or document-generator matches. Pattern matching and safety checks remain in the Rust classifier. The reviewed file is bundled into each build; it does not fetch remote lists.

`format_version` is 1. `revision` identifies the policy and must change whenever rules change, so saved messages are reclassified through the existing background queue. Explicit message and sender corrections survive this process. `type_phrases` adds every matching type using case-insensitive substring matching against the subject and unquoted visible body. Named `phrases` sets use lowercase substrings; `export_archive` requires every phrase and status sets return the first match. `patterns` uses Rust regex syntax, with case sensitivity specified per pattern. Extraction patterns return capture group 1; `code_validity` also returns a duration unit in group 2. `automated_sender` and `delivery_role_sender` match the local part of the address; `automated_sender_address` matches the full address. `delivery_headers` matches parsed header names, never body text.

Run `cargo test -p dakia-inference` after editing. The loader rejects unsupported formats, missing required patterns, unknown fields/types, invalid regexes and empty phrase sets. New type phrases take effect without Rust changes. New policy behavior still needs a classifier change and tests.

## Detection and correction

The server parses the existing MIME content and runs bounded, deterministic English and structured-metadata rules under policy `personal-first-7`. No classifier model, tokenizer, ONNX library or classifier download is required. Translation still uses its existing worker and policy. Unsupported languages without structural or sender evidence stay Personal.

Account-local outgoing-recipient evidence helps retain correspondence and personal invitations. Common automated sender addresses, including info, no-reply variants, notifications, alerts, updates, newsletters, digests, reminders, bounces and bots, are Automated even in reply threads or personal invitations. Matching uses the sender address, not its display name, and supports separated tags and common no-reply suffixes. An exact-address rule also identifies `notify@updates.notion.com` as Automated. Shared addresses such as support, billing and sales need other evidence. Mailing-list and automatic-response headers also identify automation. Explicit message and sender corrections still take precedence. Generated transactional messages are evaluated separately. Type detection is best effort and can overlap. Facts and links appear only when extracted; adding a type never creates facts. Links require a click and show their domain. No destination requests, remote attachment inspection, payment actions or calendar responses are performed.

Corrections use the existing idempotent operations endpoint:

- `inbox_kind`: optional `value` (`personal` or `automated`) and `sender` boolean. A null value clears the selected scope. An explicit sender choice also clears the selected message override so that choice applies immediately. Other individual message overrides remain authoritative.
- `detected_type`: a type `value` and `present` boolean.
- `attention`: optional boolean `value`; false dismisses, true marks, null restores automatic attention.

Precedence is message override, account-specific normalized exact-sender preference, automatic result. Dismissals and type corrections survive reprocessing. Attention is independent of type corrections. Codes use stated relative expiry where supported, otherwise a 15-minute display cutoff from receipt. This cutoff does not assert that a code is invalid. Unresolved payment failures and cancellations remain until dismissed or moved out of the inbox.

Search accepts `inbox_kind`, `email_type` and `attention=true`. Effective classifications are applied before pagination in inbox, cache previews and search. Continuations bind the filters. Browser cache namespace v2 and revisioned query keys exclude old category snapshots. Rust owns the generated TypeScript contracts in `packages/api-types`.

## Upgrade and compatibility

Migration 21 retains legacy category values and manual choices. People and Other map to Personal; Transactions, Notifications and Newsletters map to Automated. It does not fabricate detected types. Legacy category fields and operations remain available for one compatibility release.

The migration cancels old classifier jobs and skips outstanding pre-upgrade notification jobs. Existing mail is progressively enriched without creating new notification receipts. New arrivals are scheduled first; all inbox mail, including read and manually classified mail, is eligible. Opening other folders or messages requests enrichment. Policy changes requeue stale results once per account. Job identity, content revision, timeout, retries, isolation and terminal failure handling remain enforced. Body eviction retains classification details.

Notifications wait for a decision and are eligible for Personal or active Needs attention, subject to existing blocking, unread and snooze rules. Historical backfill does not replay notifications.

Before deployment, stop the application, create a matching-version administrative backup, and restore it into a separate empty directory. Keep the previous image and configuration available. Rollback after migration requires restoring the backup with the matching old image; do not point old code at a migrated store.

## Verification

The automated suites cover conservative fallback, overlapping types, HTML-only content and quotes, malformed and misleading metadata, ambiguous codes, unsafe links, expiry, correction precedence, sender/account isolation, stale jobs, read-mail scheduling, migration/restore and filtering beyond the first page. `dakia-classifier-check` is an offline deterministic packaged-runtime check.

`playwright.personal-first.config.ts` starts a disposable real backend fixture and exercises MIME parsing through durable storage and the browser. Its Chrome journey covers keyboard corrections, filters, attention, persistence and light/dark desktop and narrow layouts. Demo and persistent-cache journeys exercise their own data paths.

Real-mail detection accuracy is not measured by these synthetic checks. Conservative rules will leave some automated messages in Personal and eligible for notifications. Content-free rollout diagnostics should compare migration versions, message counts, classification status/kind counts, pending jobs and push states without logging subjects, senders, bodies, codes or URLs.
