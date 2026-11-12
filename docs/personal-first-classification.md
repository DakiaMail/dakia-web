# Personal-first classification

Dakia Web keeps three independent decisions on each message: Personal or Automated, overlapping detected types, and Needs attention. Personal is the conservative fallback while work is pending, fails, or has insufficient evidence. Reading never changes these decisions.

The inbox opens on Personal and remembers the last tab for that user. Automated has one optional type filter. All and search retain their normal scope. Tabs keep separate scroll positions and selections. Needs attention is a shared, chronological inbox query across the selected accounts; the same messages remain in their tabs. Snooze hides attention temporarily. Archive, trash and spam remove it from inbox attention.

## Detection and correction

The server parses the existing MIME content and runs bounded, deterministic English and structured-metadata rules under policy `personal-first-1`. No classifier model, tokenizer, ONNX library or classifier download is required. Translation still uses its existing worker and policy. Unsupported languages without structural evidence stay Personal.

Account-local outgoing-recipient evidence helps retain correspondence and personal invitations. Sender names, business domains and no-reply addresses alone do not decide the main kind. Generated transactional messages are evaluated separately. Type detection is best effort and can overlap. Facts and links appear only when extracted; adding a type never creates facts. Links require a click and show their domain. No destination requests, remote attachment inspection, payment actions or calendar responses are performed.

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

The local pre-release run on 2026-11-13 used Node 24.22.3, Rust 1.98.0 and installed Google Chrome. It passed 481 Rust tests (22 explicitly ignored fixtures or manual checks), 242 web unit tests, the production frontend build, and 15 worker tests (one opt-in translation check skipped). Clippy passed with only the existing Graph-module `too_many_arguments` and `items_after_test_module` lint exceptions. The focused backend classification, snooze/corrections and keyboard journeys passed. Broader Chrome checks covered paging, cold persisted folders, read stability, selection, quick actions, error rollback and responsive toolbar behavior. The ten demo journeys and pixel-parity check passed. The intermediate packaged image passed offline classification and translation smoke checks; release CI repeats packaged checks on both architectures for the published revision.
