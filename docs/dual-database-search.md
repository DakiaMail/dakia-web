# Dual-database search implementation status

The implementation passes the correctness and browser-search matrix below.
It has not been released or deployed. Current ARM64 and AMD64 development
packages pass offline inference, startup, sign-in configuration and restart checks.
The user chose delivery without waiting for the million-message stress measurement.

## November 14 integration status

Current acceptance prioritizes correct, complete results with zero degraded
searches on the normal workload. There is no latency pass/fail threshold.
Measure at least 1,000 mixed Chrome searches for each database combination and
report submission-to-visible p50, p95 and maximum latency, with cold starts
separate. Database and account timeouts remain cancellation safeguards for
outages; they must not silently turn incomplete results into complete ones.
Earlier latency targets and deadlines below describe historical runs.

Normal indexed reads now have a 10-second database safeguard and a shared
15-second account-search safeguard. Coverage and spelling checks allow 10 seconds,
and query embedding allows 5 seconds. These protect cancellation and outage
recovery, not a product speed target. The provisional header preview remains
20 ms. A timeout still marks the affected result incomplete.

Filtered semantic retrieval evaluates eligible messages before limiting the
result head. Unfiltered approximate retrieval falls back to a complete per-message
selection if duplicate passages leave the head underfilled. SQLite retains only
200 best message passages during that fallback and stops when the caller cancels;
PostgreSQL performs the per-message selection in the database. Current-state and
replay checks still validate all candidates.

The working branch includes main through `1cb0c6c`, including Microsoft accounts,
personal/automated classification, inbox filters, snooze restoration and sparse
mail-view indexes. These paths use the same native application repositories as
search. Classification filters apply before candidate limits and again when
validating current message state. The existing mailbox layout is retained.

Current migration heads are application SQLite **0040**, application PostgreSQL
**0024**, control SQLite **0011**, control PostgreSQL **0003**, search SQLite
**0023** and search PostgreSQL **0015**. Native lexical revision remains **5**.
Published main migrations retain their numbers. The unreleased search migrations
were renumbered to follow them; disposable fixtures from the earlier branch need
to be rebuilt rather than reused across those changed migration numbers.

Merged storage unit tests (109), server unit tests (182), API tests (42), and
frontend tests (286) pass. Actual Chrome layout and indexing journeys (8) pass.
Native server HTTP and operator transfer checks, PostgreSQL account/control,
filter/replay, backup, offline transfer, lexical fault/restart, completion receipt,
payload repair, attachment provenance, embedding recovery and source-validation
contracts pass. Workspace Clippy passes with warnings denied. The pinned local
E5 model retrieves the Lisbon accommodation examples without literal hotel terms.

## November 15 measured results

Each combination completed 1,000 actual Google Chrome searches over 250,000
synthetic messages across five accounts, with 950,750,000 bytes of extracted text,
1.5 million lexical chunks and one million vector chunks. All 4,000 searches
returned complete coverage with zero degraded responses and zero rendering
failures. Query embeddings were uncached; the browser reloaded every 24 queries.
Submission-to-visible measurement ends after the expected result rows and two
animation frames. Relevance and strict-query behavior are covered separately by
the model, filter, phrase, ranking, pagination and authorization contracts.

| Application storage | Search storage | Median | p95 | Maximum | Process restart to search-ready |
| --- | --- | ---: | ---: | ---: | ---: |
| SQLite | SQLite | 112.5 ms | 216.1 ms | 414.9 ms | 59.42 s |
| PostgreSQL | SQLite | 181.2 ms | 283.3 ms | 1,316.6 ms | 71.00 s |
| SQLite | PostgreSQL | 280.7 ms | 1,248.0 ms | 1,735.1 ms | 1.19 s |
| PostgreSQL | PostgreSQL | 397.8 ms | 1,570.7 ms | 6,587.5 ms | 1.18 s |

Measured on an Apple M1 Max with 10 CPU cores and 64 GiB RAM. PostgreSQL 17.11
and pgvector ran locally in Docker with 10 CPUs and about 16 GiB available.
Owned compilation, other test runs and large-graph construction were not running
during these measurements. Restart measurements reopen retained indexes and include
SQLite graph/metadata loading; they are separate from the warm browser figures.
They do not represent a reboot with an empty operating-system disk cache.
The production mail UI can remain available while semantic search warms up.

The final source passes all 109 default storage tests and 182 default server
tests. PostgreSQL-specific checks cover all backend combinations, generic
prepared plans, long-message passage deduplication, current source validation,
strict filters, restart and cancellation recovery. Local E5 tests include the
Lisbon accommodation question without literal hotel terms and quoted constraints.
Workspace Clippy passes with warnings denied.

A repeated-query PostgreSQL failure was traced to converting a text query vector
for every passage in generic prepared plans. The query now materializes that
conversion once. Exact filtered selection evaluates metadata once per message,
then deduplicates passages before limiting the head. These changes preserve the
same ranked rows, excerpts and distances; they do not cap eligible discovery.

The retained SQLite 250,000-message fixture occupies about 14.47 GiB including
application files and vector graphs. Its search database contains about 10.96 GiB
of used pages, plus a 1.57 GiB vector snapshot. These are measured fixture sizes,
not a forecast for arbitrary real mail or attachment distributions.

The one-million-message fixture durably contains 3,803,000,000 extracted text
bytes, six million lexical chunks and four million vector chunks. Its search
SQLite file is 47.07 GiB (46.80 GiB of used pages), before the vector snapshot.
The initial graph build was stopped at the user's request to deliver the completed
results, after about 99 CPU minutes; no completed one-million-message search
timing is claimed. Its 2 h 42 min wall time includes deliberate pauses for the
browser measurements and must not be presented as uninterrupted indexing time.
The durable fixture is preserved, but its disposable vector graph must be rebuilt
before a future one-million-message query measurement.
Fixture construction reused model output only for identical verified passage
contexts; that construction time is not a real-mail embedding-throughput result.

Earlier measurements and deadlines below are historical. The numbers above
supersede their latency acceptance claims. No further latency optimization is
required by the current correctness-first acceptance scope.

## Resuming retained scale fixtures

The scale fixture writes `scale-fixture.json` after all messages and extracted text
are committed, before embedding work starts. Keep its printed `SCALE_ROOT` in a
durable directory. After an interrupted run and expiry of its outstanding worker
lease, run `release_mailbox_search_replay` with `DAKIA_BENCH_REUSE_ROOT` set to that
directory and `DAKIA_BENCH_FINISH_INDEXING=1`. The runner retains existing vectors,
finishes durable jobs and verifies completed coverage before measuring queries.
Fixture completion time is reported separately; rerun without that flag for a
process-startup measurement. This path accepts only the synthetic scale fixture.

SQLite embedding claims now use the existing ordered pending/history queue
indexes. Plan assertions reject a document-first scan or a temporary ordering
sort, while the recovery contract retains lease, revision and fairness checks.

## Current behavior

`DAKIA_DATABASE_URL` selects application storage independently:

- Unset or empty: existing SQLite control and account files in `DAKIA_DATA_DIR`.
- `sqlite:/absolute/path/control.sqlite`: selected SQLite control file, with account
  files and durable blobs retained in `DAKIA_DATA_DIR`.
- `postgresql://…` or `postgres://…`: native `dakia_control` and `dakia_mail` schemas.

`DAKIA_SEARCH_DATABASE_URL` selects the search store:

- Unset or empty: follows the application backend, with separately managed search
  tables in PostgreSQL or `DAKIA_DATA_DIR/search/index.sqlite` for SQLite.
- `sqlite:/absolute/path/index.sqlite`: independently located SQLite search store.
- `postgresql://…`: PostgreSQL search store in the `dakia_search` schema.

Changing a connection URL does not transfer application data. Use offline
export/import with the same credential key before switching an existing store.

Treat connection URLs as secrets. Do not put them in logs, Git, or shell history.
PostgreSQL provisioning requires pgvector 0.8 or later and `pg_trgm`. Extension
availability and vector version are checked before search migrations. Migration
failure or an unavailable search service leaves mail startup and provider search
available; background jobs retry the search connection.

| Application storage | Search storage | Status |
| --- | --- | --- |
| SQLite | SQLite | Implemented; contract tested |
| SQLite | PostgreSQL | Implemented; contract tested with isolated PostgreSQL 17 |
| PostgreSQL | SQLite | Implemented; Store and compiled-server HTTP contracts passed |
| PostgreSQL | PostgreSQL | Implemented; Store and compiled-server HTTP contracts passed |

Control persistence now has native SQLite and PostgreSQL implementations behind
a typed repository. Authentication, invitations, sessions, OAuth flows, account
ownership, encrypted credentials, account revisions, search cursors, subscriptions,
and rule definitions use this boundary. A shared contract exercises concurrent
owner setup, invitation consumption, account updates, rollback, and authorization.
PostgreSQL control snapshots stream a repeatable-read transaction into the existing
SQLite backup format, preserving IDs and ciphertext. Account snapshots stream
from a repeatable-read PostgreSQL transaction into the same portable format,
including queue ordering, MIME bytes, revisions and compatibility records.
Offline import refuses populated destinations, verifies the key before writing
an external database, pauses jobs, and retains a startup guard on interruption.
PostgreSQL offline operations hold an exclusive database advisory lease; online
application instances hold shared leases, including instances on other hosts.

The account repository now implements drafts, submission idempotency, delivery
transitions and leases, immutable send snapshots and MIME, filing, event cursors,
provider documents, unsubscribe claims, attachment records, and composition lists
for both engines. PostgreSQL tables have account-scoped keys and ownership foreign
keys. Transactions validate and lock current ownership, serialize account writes,
and maintain a separate event sequence per account. Removal cascades through these
tables. SQLite installations retain their account files and use the same typed
methods. An additive scheduled-send projection avoids parsing opaque unsubscribe
receipts as JSON and preserves existing scheduled messages during migration.

Mailbox ingestion and mutation decisions now share a composable transaction
implementation. The native PostgreSQL tables project indexed sender, date,
folder, flags, category, operation target, and provider epoch fields. Message,
content, and epoch changes publish versioned search receipts in the application
transaction. Old acknowledgements cannot delete newer receipts. Empty memberships
and changed IMAP epochs remove stale source text and retain delivery tombstones.
SQLite write transactions obtain writer admission before reading revisions.

Folder traversal, incremental checkpoints, inbox projections and coverage,
first-arrival receipts, and mutation workers now have native implementations for
both engines. They retain generation guards, complete-scan-only pruning, limited
coverage, late foreground discoveries, arrival deduplication, per-message mutation
ordering, bounded retries, and cancellation before claim. Interrupted mutations
are not automatically replayed. These native repository contracts do not yet
establish full application parity or enable the PostgreSQL application option.

Classification and translation jobs, notification workers, snooze wakeups, rule
status views, and content-reference recovery also use shared repository methods.
Native PostgreSQL model queues index active work separately from completed
history. Both engines retain content revision and deadline checks, manual classification
protection, durable retry limits, notification send-time
checks, and reference observation fences. Empty reference-recovery polls do not
consume the account admission window; admitted lookups retain the daily message
cooldown. Concurrent updates to one sender rule retain its original identity.

Thread graphs, verification proofs, provider identity aliases, received-cache
metadata, starred navigation, and recipient evidence now have native repository
implementations. PostgreSQL thread edges are account-scoped and backfilled from
retained headers during migration. Topology changes invalidate proofs; flags,
display changes, and refreshed topology timestamps do not. Cache files use the
shared filesystem path, with metadata/eviction behind the repository. Cache
eviction does not delete indexed text or its delivery receipts. Recipient ranking
counts distinct evidence, chooses the newest matching record deterministically,
and preserves the existing ASCII case-insensitive address identity on both engines.

Sender cleanup also uses the native account repository. Frozen targets, replay
identity, overlapping jobs, pending moves, dismissal, and status projections share
one transaction implementation. Sender counts and pages observe one snapshot;
PostgreSQL uses indexed sender identity and compatible timestamp interpretation.

Header reconciliation, content acquisition, retry queues, exclusion mappings,
source inventory, delivery reads and acknowledgements, metadata previews, account
initialization/removal guards, and live
result validation now use the account repository on both engines. PostgreSQL
maintains per-message eligibility and incremental account counters. Current-state
validation reads an entire destination page, including snoozes, in one source
snapshot instead of issuing a query for every result. The compiled server also exercises these paths through authenticated HTTP on
all four combinations; release-wide recovery and performance acceptance remains open.

Search deliveries are bounded, transactional batches from a durable account
outbox. The source transaction records changes, the destination accepts newer
versions idempotently, and the source acknowledges only after the destination
commits. Successful acknowledgement releases extracted body, participant, MIME
recipient, and attachment text from application staging; the selected search store
retains that content. Metadata patches preserve previously delivered text. A newer
source receipt prevents an older acknowledgement from clearing its staged payload.
A changed destination incarnation requeues released payloads for acquisition.
Tombstones prevent delayed delivery from resurrecting deleted content.
Current account ownership, provider identity, pending changes, and excluded
memberships are rechecked before returning a destination result. The server
removes the obsolete in-account FTS index and its triggers, so a damaged search
index cannot block mailbox writes through those triggers.

The index includes metadata, decoded MIME participants, both body alternatives,
attachment filenames, and text supported by anydoc. Unsupported or unreadable
attachments produce explicit gaps while retaining successful text. Indexing does
not retain attachment binaries in the MIME cache. Extraction and work batches
remain bounded; there is no aggregate search storage quota.

The local E5 runtime is implemented separately from retrieval. It loads the
pinned quantized multilingual-e5-small ONNX model and tokenizer from
`DAKIA_SEARCH_MODEL_DIR` (source default
`crates/inference/assets/multilingual-e5-small`). The build pins the manifest,
verifies every asset hash, applies query/passage prefixes, masks padding during
mean pooling, and normalizes vectors. No runtime download or inference service is
used. The runtime returns UTF-8 passage cursors with overlapping token evidence;
long content can resume without silently truncating later passages. Work uses
bounded text windows and batches. Passage inference is independent of batch
composition because quantized batches otherwise changed vectors.

`python3 scripts/fetch-search-model.py` obtains only pinned public assets.
`dakia-search-model-check` loads copied assets and ranks synthetic passages on
CPU. Both image definitions and the bundle builder include these assets and the
smoke binary. Architecture-specific image execution is still an acceptance gate.
A background worker now discovers and leases jobs in the selected search store,
reads bounded UTF-8 windows of retained text, and commits each vector together with
its passage cursor. It resumes after interruption, rotates partial historical
messages fairly, prioritizes newer arrivals, backs off failures, and continues
local work through provider reconnects. Search-storage transactions fence content
versions, model revisions, lease tokens and cursor positions. Flags-only changes
preserve vectors. Trash, source identity changes and epoch invalidation revoke
obsolete work. SQLite records durable accelerator changes.

SQLite now maintains per-account/model USearch graphs from authoritative stored
embeddings in bounded 256-row turns. An atomic, checksummed snapshot manifest
commits before SQLite replay receipts are acknowledged. Missing or corrupt
snapshots reconstruct locally; a consumer behind the durable replay floor also
reconstructs. Acknowledged receipts are reclaimed in bounded transactions.
Queries do not rebuild graphs on their request path. Readiness checks use an
indexed account/model replay scope, so another account's pending vector changes
do not make a caught-up account incomplete. A consumer below the replay floor is
fenced until reconstruction finishes. Native PostgreSQL vector
retrieval uses pgvector HNSW with bounded iterative scans. Both paths return the
best passage per message and revalidate current ownership, source identity,
Trash/Spam membership, pending application writes and strict structured filters
against application storage. The packaged search-model smoke also exercises
native vector serialization and retrieval using synthetic model vectors.

Unified `/search` now uses local query embeddings and reciprocal rank fusion.
A dedicated bounded CPU lane isolates query inference from passage backfill;
loading, warmup and retries happen outside requests. Its pinned model is verified
before readiness, its queue is finite, obsolete queued requests are skipped, and
failures leave lexical and provider results available. Quoted text survives in a
shared query representation and remains a strict constraint on semantic hits.
The browser preserves relevance order and uses matching passages when available.
Account collectors run concurrently under one request deadline. Lexical and
semantic retrieval also run independently and retain completed positives when
their sibling stalls. Native PostgreSQL lock-injection contracts exercise both
directions. A missing progress-counter table also leaves both retrievers usable,
while preventing a false completeness claim.

Progress waits for the current
message's lexical delivery and embedding completion. Search storage emits a
durable reverse-outbox receipt; application storage validates its destination
identity, publication version, source, model revision and current eligibility
before counting it. A later mailbox change invalidates that receipt atomically.
Counters are maintained incrementally and read from one application snapshot.
Backlog counts each unfinished eligible message once, plus distinct ineligible
cleanup work. Delayed acknowledgements cannot delete newer receipts or lower the
source delivery version. Upgrade backfill republishes bounded metadata pages
while preserving retained text and vectors.

Both databases now store bounded canonical lexical chunks. Native FTS5 and GIN
indexes select token candidates; shared canonical sequence checks verify phrases
without PostgreSQL's capped position lists. Per-unit field weights are shared
across backends, with date and stable message identity breaking ties. Small
messages publish their chunks atomically with retained text. Larger documents
advance through durable 64 KiB work windows and overlapping chunks, independently
of model availability. Lexical checkpoints and chunks commit together; account
failures persist retry backoff. Completion receipts require the current lexical
revision as well as the model revision. Upgrade counters immediately exclude old
lexical proofs and bounded metadata publication preserves retained vectors.

Text searches use shared weighted relevance with keyset pagination.
Unquoted words may match separately; quoted groups remain phrases. Folder views
remain in date order. The current fusion prefix merges 512 lexical candidates
and 200 semantic candidates globally, with deduplication and recency/identity
ties. The prefix is saved in owner-bound versioned cursors and revalidated against
current application ownership, source identity, headers, exclusions and strict
filters before presentation. Lexical keysets continue after the prefix. The
concurrent mutation matrix and large-scale relevance/latency acceptance remain
open; this is not a claim of full search acceptance. Bounded ordinary-word
spelling expansion is implemented. Exact subject, participant and contiguous phrase boosts are shared
across both lexical repositories before the candidate limit.

Trash and Spam/Junk role mappings exclude their messages, including alternate
folder memberships. Direct folder browsing continues through the existing mail
routes. Index outages preserve valid cached header positives and return an
independent provider continuation per incomplete account. The client starts those
lanes without waiting for local pagination. Slow or failing provider accounts do
not block other lanes or local results. Provider-only results carry explicit
semantic/attachment limitations and bounded lower-weight reciprocal rank scores.
The client merges identities without dropping valid indexed attachment/semantic
positives when provider literal coverage finishes. Existing sequential
continuations remain usable by clients that do not consume the optional lanes.
Provider roots stay pinned through index refreshes. New accounts can join without
restarting existing work; caught-up accounts stop their provider lanes. Temporary
failures retry with bounded backoff, expired/version-changed cursors obtain fresh
roots, and authentication failures require explicit retry or reconnection. Query
changes cancel obsolete requests. Shared query units emit separate IMAP `TEXT`
criteria for ordinary terms and one criterion for each quoted phrase, preserving
the query interpretation on provider fallback.

Account indexing status is authorized at `GET /api/accounts/{account}/indexing`.
A thin progress bar appears under each account after two seconds of work. It is
indeterminate during discovery, shows known progress afterward, pauses on an
outage, and disappears when the backlog drains. Terminal content gaps remain in
the accessible account description. Source discovery and delivery counters are maintained incrementally in the
application database. The selected search database separately persists terminal
text coverage, byte counters and lexical and embedding-stage counters. Current
lexical and embedding completion proofs gate endpoint and sidebar completion.
The denominator includes all discovered eligible mailbox metadata. Completion is
counted only after destination delivery commits; invalid IMAP epochs and
Trash/Spam memberships are excluded. An unavailable selected search database
returns paused status using source inventory, without blocking mail startup.
Connection warming and retries run in the background. Acquisition pauses during
search outages, retaining pending work and already staged payloads.

## Remaining release gates

- Complete operational and packaged acceptance for both application backends.
  Native control/account repositories, independent connection selection, account
  lifecycle, portable snapshots and offline transfers are implemented. The four
  combinations pass Store API and compiled-server HTTP smoke contracts, including
  authorization, indexing status, body search, and existing account actions.
  These do not replace the full fault matrix or packaged architecture checks.
- Make large destination rebuilds bounded, and add cross-process destination
  incarnation fencing and complete component health/repair acceptance. Source
  receipts now confirm model stages per message; cross-process model mismatch
  cannot claim current semantic completion. These contracts do not establish the
  complete large-scale outage and corruption matrix.
- Finish large-scale hybrid relevance, concurrency and fault acceptance. Query
  embeddings, fusion and strict phrase checks are now active. Validate accelerator
  recovery and query model outages under realistic load; a SQLite graph behind
  the durable replay frontier is explicitly degraded rather than complete.
- Validate indexed metadata query plans and broad filtered semantic recall at
  scale. Scalar flags, dates, categories, folder memberships and snoozes now
  constrain both retrievers. Participant/subject substring predicates now use
  native trigram indexes and still need scale acceptance. Attachment spans and
  semantic file identities are implemented;
  legacy extracted payloads without spans retain unlabelled excerpts.
- Complete independent-store repair fault/multi-process acceptance and packaged
  backend smoke tests. Native repair tools now cover both search backends.
- Measure the approved 250,000-message, five-account workload with more than
  512 MiB of extracted text, uncached query embeddings, and realistic browser
  network time. Report submission-to-render latency for all four combinations
  without a latency gate. Run the separate million-message growth test.

No hybrid-search performance claim follows from the lexical contract tests.
Search coverage follows the saved folder scan generation and provider epoch.
Completed scans do not expire with wall-clock age. A scan reset, continuation,
limited scan, missing folder, or mismatched epoch requires provider fallback.
Routine folder discovery and Inbox refresh keep their existing schedules;
those schedules do not make a completed offline search index incomplete.

## Verification commands

```sh
cargo test -p dakia-storage --lib
cargo test -p dakia-server --lib
npm test
npm run build
CHROME_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  npm exec --workspace @dakia/web -- playwright test browser/indexing-progress.spec.ts
```

The ignored `independent_postgres_search_delivery_and_exclusion_contract` test
requires `DAKIA_TEST_POSTGRES_URL` pointing to an isolated disposable database with
pgvector available. Never point this fixture at production.

The ignored `postgres_control_auth_contract` test requires a separate disposable
database named `dakia_control_test`, selected by
`DAKIA_TEST_CONTROL_POSTGRES_URL`. It resets only that database's `dakia_control`
schema, then runs the shared control contract and backup restoration checks.

The ignored `postgres_account_submission_contract` test requires a disposable
database named `dakia_account_test`, selected by
`DAKIA_TEST_ACCOUNT_POSTGRES_URL`. It resets that database's `dakia_mail` and
`dakia_control` schemas. It tests real concurrent writes, atomic rollback, owner
boundaries, account-local keys and events, immutable MIME, attachment retention,
scheduled views, mailbox replay, concurrent ingestion, pending intent, rule
arrival receipts, complete/partial scans, generation resets, arrival rollback,
inbox pagination, versioned search receipts, epoch changes, mutation ordering,
retry budgets, cancellation, removal cleanup, and retry after reconnecting.
Worker coverage also includes classification backoff, unread cancellation,
manual categories, translation ownership and rollback, notification eligibility,
send-time changes, retry budgets, snooze wakeups, rule views, and reference
recovery cooldowns. A database trigger fails reference promotion after its
pointer write, proving rollback of both the pointer and its observation token.
Sender cleanup contracts cover frozen target drift, replay and overlap guards,
concurrent requests, rollback after an operation is written, timestamp offsets,
exclusion mappings, consistent count/page views, and account isolation.
The acquisition contract covers normalized body/participant/attachment staging,
backoff, stale-source rejection, empty memberships, alternate exclusions and
restoration. Set `DAKIA_TEST_POSTGRES_URL` as well as the native application fixture
URL to exercise both selected search targets in that contract. These repository
checks isolate persistence; the compiled HTTP contract separately covers the
server entry point.
The graph/cache contract covers topology-only invalidation, edge replacement,
epoch changes, bounded verification proofs, identity allocation under concurrency,
starred cache priority, cache/search independence, and recipient evidence replay.
The PostgreSQL fixture also reopens a pre-graph schema and verifies migration
backfill from retained headers without losing cache records. That fixture retains
opaque provider receipts too: the migration materializes the header subset before
decoding JSON, preventing query-planner reordering from parsing unrelated state.

The embedding PostgreSQL contract requires a separate disposable database named
`dakia_embedding_test` selected by `DAKIA_TEST_EMBEDDING_POSTGRES_URL`. The storage
fixture resets only its `dakia_search` schema. The server's ignored
`postgres_worker_persists_real_embeddings` fixture uses the same selected target
and requires the pinned E5 assets. Its local source account is disposable.

Chrome screenshots from the indexing review are under
`apps/web/artifacts/indexing/` (ignored local evidence). The browser scenario
covers both themes, desktop and narrow/short layouts, keyboard account navigation,
discovery, determinate progress, paused work, retry errors, completion, and reload.
It includes provider reconnects, reduced motion, focus preservation and removal
of reserved space on completion. Completed progress stops polling; existing
mail/index events immediately invalidate the status without cancelling slow
requests. Rendered fill-to-track contrast is 3.48 in light mode and 4.87 in dark
mode. The thin bar keeps the same contrast while paused.

## Recorded checks

- Rust server library: 144 passed; explicit integration, network and real-model
  fixtures are ignored in the ordinary suite and run separately as described below.
- Rust storage library: 60 passed; thirteen explicit benchmarks/PostgreSQL fixtures
  ignored. Integration suites passed another 42 tests.
- HTTP API: 37 tests passed. All workspace targets compile.
- Compiled native server HTTP contract passed on all four database combinations,
  including persisted body matches, progress, cache previews, authorization,
  translation queues, mutation idempotency, attachment quotas, and SSE shutdown.
- Portable backend round trip passed with the same encryption key, preserving
  credentials, IDs, drafts, submission idempotency and attachment bytes. New login
  registration succeeds after import. A forced import failure rolls back account
  rows and blocks startup from both the original and another data directory.
- Populated native account export/import compares every durable column, event and
  queue ordering against the portable archive, including worker and legacy rows.
- The actual `dakia-admin` binary passed SQLite export, PostgreSQL import/export,
  and SQLite import. A live PostgreSQL app lease prevents offline export.
- Compose configuration validates with empty optional database environment values,
  preserving the SQLite default.
- Both native control contracts passed separately, including account revision
  concurrency, ownership, invitation rollback, session-bound OAuth, rule access,
  ciphertext preservation, and backup restoration through the Store API.
- The native PostgreSQL account contract passed separately, including concurrent
  saves, idempotent submission retries, atomic unsubscribe rollback, immutable
  MIME and snapshots, attachment scope and retention, scheduled views, account
  removal cascades, and retry after reconnecting. The SQLite contract and the
  scheduled-projection migration test passed in the storage suite.
- The shared account contract also covers mailbox ingestion, rule arrivals,
  reference observation fences, scan/checkpoint atomicity, late discoveries,
  limited coverage, concurrent resets and claims, versioned publication, epoch
  invalidation, mutation ordering/retries, and interrupted-work recovery.
- Both native account contracts cover classification and notification workers,
  rule state, translation persistence, and reference recovery. The native control
  contract verifies one stable rule ID under concurrent writes.
- Graph/cache/recipient contracts passed on SQLite and PostgreSQL, including
  migration backfill, owner isolation, and cache eviction without search loss.
- Native application repository delivery contracts passed with both search
  targets. They exercise replay after a lost acknowledgement, a newer payload at
  the acknowledgement boundary, text release, incarnation changes, metadata
  previews, batch validation, and snooze filters. The PostgreSQL source contract
  compares incremental counters with underlying records after changes and checks
  account removal cascades for the new inventory tables.
- The outage server fixture verifies nonblocking configuration, paused status,
  acquisition suspension, provider fallback, and recovery after storage returns.
- The independent payload contract passed for SQLite and PostgreSQL destinations,
  including restart, repair, metadata patches, a concurrent acknowledgement race,
  retained attachment/participant matches, and MIME eviction independence.
- Both PostgreSQL search contract tests passed separately against a disposable
  PostgreSQL 17/pgvector container, including phrase and relevance pagination.
- Inference: all 23 tests passed with real pinned classifier and E5 assets,
  including multilingual relevance, batch-independent vectors, invalid assets,
  masked pooling and later-passage cursor coverage.
- Durable embedding queue contracts passed on SQLite and PostgreSQL. They cover
  concurrent claims, resumable byte cursors, active and expired leases, backoff,
  model changes, flags-only preservation, Trash/restoration, provider epochs,
  per-account isolation and fair rotation of partial history. An injected failure
  after chunk insertion rolls back its vector, accelerator receipt and cursor.
- Completion contracts passed on all four application/search combinations. They
  cover disjoint stage populations, source and target persistence failures, lost
  and delayed acknowledgements, current model fencing, metadata-only updates,
  version backfill without inference, terminal gaps, restart, alternate Trash
  membership, repeated epochs, exclusion-map restoration and account removal.
  A semantic bookkeeping outage preserves healthy lexical candidates. Endpoint
  tests verify active local indexing through provider reconnect, retry errors,
  and completion only after source persistence.
- The real E5 worker passed against both SQLite blobs and PostgreSQL vectors:
  later passages survive persistence and rank by meaning, without MIME retention
  or provider access, including while the provider account needs reconnection.
- Vector retrieval contracts passed for all four application/search combinations,
  including current ownership, strict filters, account removal and alternate
  Trash membership before graph replay. SQLite contracts also cover snapshot
  restart, corruption, loss, replay, a lagging consumer and failed persistence
  without receipt acknowledgement. Real E5 retrieval chooses the relevant later
  passage through both repositories.
- The release-build model smoke passed with native USearch serialization and
  retrieval. Workspace Clippy passed with warnings denied. Linux image execution
  and large-scale vector recovery/fault tests remain unverified.
- Native E5 smoke passed. Release-build CPU-only query embedding timing on an
  Apple M1 Max, 100 unique short synthetic queries: p50 3.130 ms, p95 3.322 ms,
  maximum 3.407 ms. This excludes cold model load, database/vector work, network
  and rendering. It is not the required end-to-end hybrid acceptance benchmark.
  Reproduce with `cargo run --release -p dakia-server --bin dakia-search-model-check -- --timing`.
- Release-runner tests: 20 passed. Search asset checks and explicit real-model
  tests are included in the runner; this does not establish a release-ready build.
- Web unit tests: 222 passed. Production web build passed.
- Google Chrome: 10 affected indexing/search scenarios passed; screenshots for
  both themes and desktop/narrow layouts were visually reviewed.
- Offline repair: six tests passed using the pinned SQLite 3.54.1 engine,
  including the attachment-aware schema and rollback behavior.
- Formatting and whitespace checks passed.

These checks cover implemented paths. They do not establish unified hybrid search,
complete backend fault parity, packaged Linux acceptance, or the 200 ms
end-to-end requirement.

### Canonical lexical projection verification

The long-message contract covers a phrase after 18,000 repeated words, repeated
lexemes beyond native position-list limits, Unicode and participant-only matches,
more than 1 MiB of attachment text with a match at its tail, and phrases confined
to one field. It interrupts and reopens the store, injects a chunk/checkpoint
transaction failure, verifies rollback and persisted retry backoff, and checks
that flag changes preserve chunks. Completion stays at zero until both lexical
and embedding stages have finished. These contracts use synthetic vectors to
isolate stage persistence; they are not semantic relevance or hybrid latency tests.

All four application/search combinations passed these recovery checks. The shared
completion contract also invalidates legacy lexical proofs, republishes them
without re-inference, rejects incompatible lexical revisions, and leaves future
revision receipts for their producing runtime to acknowledge. Search continuation
version 3 rejected old native-rank cursors after the shared-score transition;
version 4 added fusion snapshots and independent provider lanes. Version 5
changes lexical ranking weights. Version 7 aligns provider recipient scope with available To/Cc/Bcc. Version 6 freezes spelling
expansions per account. Both reject older score keysets with `refetch_required`.

The broad storage suite passed 106 tests and the server suite passed 182 tests,
with environment-dependent checks reported separately. The compiled-server HTTP
contract passed all four backend combinations. These checks do not satisfy the
remaining workload, packaging, repair, and submission-to-render latency gates.


### Hybrid query and provider lane verification

A real pinned E5 endpoint check returns a parcel passage without literal
"package delivery" terms and preserves that phrase when quoted. The bounded
query-lane contracts cover cached and uncached inference, invalid inputs, queue
capacity, cancellation, failure and retry. A 550-message fixture exercises fusion
pagination, deduplication and live Trash exclusion without provider requests on
caught-up searches. Native SQLite/PostgreSQL combinations pass hybrid, phrase,
owner/query binding and provider continuation contracts.

Chrome checks cover light/dark themes at 1480x820 and 390x568, independent provider
arrival while a local page is held, deduplication, relevance order, excerpts,
composition Enter, normal Enter, focus retention, Tab order, Space selection and
reload. The existing indexing indicator remains independent of these queries.
These are disposable fixtures, not the required 250,000-message latency workload.

The web suite passes with the host Node built-in experimental web storage disabled:
`NODE_OPTIONS=--no-experimental-webstorage npm test -- --run`. This selects jsdom's
browser storage for the existing image-preference tests; no preference code was
changed to accommodate the host runtime.

## Current hybrid verification boundary

The focused hybrid contracts cover 550-result frozen-prefix pagination and live
Trash exclusion, account-source failure, missing progress bookkeeping, a stalled
search connection, and independent PostgreSQL lexical/vector table stalls.
Independent application/search backend contracts pass using synthetic vectors.
The actual pinned CPU E5 runtime also retrieves English delivery passages for
Estonian, Urdu and Japanese queries, with strict phrase constraints preserved.
This small relevance fixture does not establish large-mailbox retrieval quality.

Chrome exercises desktop and narrow layouts in both themes, immediate Enter,
IME boundaries, focus retention, keyboard selection, provider-lane independence,
and local pagination while another provider waits. Client recovery tests cover
temporary retries, stale roots, new account lanes, cancellation and authentication
failure boundaries. These use disposable synthetic data.

The required 250,000-message, five-account, 1,000-search release benchmark has not
completed. There is no verified p95 submission-to-visible hybrid latency. A debug
correctness run under competing fixture load exceeded the internal request
budget and correctly reported degraded coverage; an isolated correctness run
passed. Neither result satisfies the release performance gate. Remaining
scale-level filter query plans and recall, the complete fault matrix, and
packaged architecture checks remain open.

## Provider calendar dates

Provider observations now preserve the original `INTERNALDATE` calendar day as
optional `Message.received_date`, separately from the UTC `received_at` instant.
`since`/`after` are inclusive and `before` is exclusive, matching the provider
query. Local date-only, lexical and hybrid searches use the selected search
repository's indexed day projection. Current source validation reapplies the
same bounds to vector candidates and frozen pages. Unknown days are excluded
from local positives and prevent a completeness claim, keeping provider fallback
available. The missing-day check uses a partial index rather than a mailbox count.

Additive application migrations restart existing folder metadata discovery only
when legacy messages lack the original day. They preserve IDs, scan arrival
boundaries and receipt history; ordinary bounded background traversal acquires
the missing metadata. The SQLite upgrade contract checks a real reopen and ensures
the upgrade does not restart twice. Native PostgreSQL tests exercise the same
upgrade SQL with retained messages and boundaries. Indexed date queries are
contract tested across the four application/search combinations. Filter-aware
ANN recall and the full large-mailbox latency gate still require acceptance.

## Exact lexical evidence

The shared lexical score now adds evidence for a complete normalized subject,
a participant name or address at a stored identity boundary, and a contiguous
canonical phrase. Subject, participant and body phrase weights differ, and a
phrase contributes once per message rather than once per repeated chunk. These
bonuses are calculated in the native SQLite/PostgreSQL query before selecting the
lexical head, preserving keyset ordering for the remaining mailbox. Recency and
stable identity still break ties. Folder-only views retain date order.

Contracts on both search databases compare exact subjects against newer partial
subjects, Cc participant names against generic subjects, adjacent words against
separated words, and full addresses against fragments of other addresses. They
also traverse score keysets and verify identical ordering. PostgreSQL uses an
additive binary-collation order index, so case, punctuation and Unicode identity
ties agree with SQLite and the shared ranker regardless of database locale.
Server contracts
exercise the changed scores on all application/search combinations and reject
older cursor versions instead of applying old scores to new ranking rules.
These checks establish the functional ranking cases, not large-mailbox quality
or the required release latency. Bounded typo expansion is described below.

## Bounded spelling expansion

Ordinary alphabetic words of 4–32 normalized code points (at least four letters)
can match a single insertion, deletion, substitution or adjacent transposition.
Short words, numbers, addresses, quoted text and structured filters stay literal.
Up to 16 ordinary query units expand, with at most 512 dictionary candidates and
eight additional variants per unit. Native SQLite/PostgreSQL lexical queries
preserve conjunction across units and apply a lower per-field weight to variants.
Exact subject, participant and phrase bonuses still use the original query.

Spelling dictionaries are account scoped. Canonical chunk inserts update word
references and deletion signatures in the same transaction as lexical indexing.
The last reference removes the word and its signatures; Trash, deletion and
account removal therefore remove this derived evidence as well. Foreign-key
lookup indexes keep cleanup scoped to the affected word. Native PostgreSQL uses
`pg_trgm` similarity over the bounded signature candidates. Shared Unicode edit
checks define the accepted variants on both backends; non-ASCII candidates also
retain the locale-independent shared check.

Lexical revision 3 adds these projections. Existing text is revisited through
bounded lexical work, preserving durable embeddings. Source progress excludes
older lexical completion proofs until the new work commits and its guarded
receipt is accepted. Newer lexical receipts remain unacknowledged by older
runtimes. A newer source profile cannot be downgraded by an older lexical worker.

Search continuations freeze each account's expansion choices, including literal
fallback choices after a lookup failure. Spelling lookup has its own bounded
request budget; failure leaves lexical and semantic retrieval usable and starts
provider fallback with degraded coverage. Providers continue to receive the
original literal query. Tests cover Unicode edits, first-character changes,
phrases and filters, account isolation, real reopen, excluded-content cleanup,
550-result typo pagination, and native PostgreSQL spelling-table stalls. Upgrade
contracts verify old proofs stop counting, retained text rebuilds the dictionary,
and vector IDs and values remain unchanged on both search databases. Full-scale
latency, retrieval quality and the remaining filter/repair gates are still open.


## Filtered retrieval and metadata replay

Search storage projects unread/starred/category fields and indexes dates, snoozes
and relational folder memberships. The application transaction publishes every
member of a thread when its snooze is created, changed, removed or expires. Both
native search paths apply structured constraints before selecting semantic hits;
current application ownership, source identity and exclusions are checked again
before presentation.

SQLite keeps current filter metadata beside its rebuildable graphs. Metadata-only
changes replay without replacing unchanged vectors. A restart rehydrates metadata
in pages of at most 256 identifiers and an 8 MiB payload budget before enabling
filtered graph queries. Snapshot watermark updates reuse unchanged graph files;
snapshot acknowledgement checks the destination incarnation before advancing the
replay floor. Long metadata rehydration across competing processes still needs
stress acceptance.

Small structured cohorts use exact cosine distance over at most 4,096 qualifying
chunks and 6 MiB of authoritative vectors. Broader cohorts use USearch's native
filter callback. A nonempty cohort with no returned ANN hits degrades explicitly.
PostgreSQL applies metadata predicates inside its vector candidate query, with
bounded strict iterative HNSW scans. Filtered ANN can require additional scanning;
see the upstream [filtering](https://github.com/pgvector/pgvector/blob/v0.8.2/README.md#filtering)
and [iterative scan](https://github.com/pgvector/pgvector/blob/v0.8.2/README.md#iterative-index-scans)
documentation. These bounds do not establish recall or the latency target at scale.

The recipient filter uses available To, Cc and Bcc values. Provider fallback emits
a grouped OR of the corresponding IMAP search keys, as specified in
[RFC 9051 section 6.4.4](https://www.rfc-editor.org/rfc/rfc9051.html#section-6.4.4).
Other filters remain conjunctive and literal. Cursor version 7 rejects continuations
created before this provider-scope change.

## Retained content and attachment provenance

Encrypted and resource-limited bodies retain readable participant headers and
attachment filenames. Successfully acquired bounded body prefixes remain
searchable alongside an explicit resource gap. Ciphertext is never indexed. Encrypted MIME subtrees are isolated without
discarding readable sibling bodies or attachments.
Extraction records each attachment's byte range in normalized UTF-8 text, its MIME
attachment ordinal, filename and processing state. Semantic retrieval picks the
best chunk per message before materializing metadata, then attaches the filename
and excerpt of the largest overlapping file range. Old payloads without reliable
ranges return their passage without inventing a file identity. Metadata-only
publications preserve extracted text and provenance.


The native filtered-search contract now passes for all four application/search
combinations. Its rare eligible vector lies beyond 1,024 nearer excluded
neighbors. It also exercises thread snooze publication, flags, source filtering,
metadata replay and restart. The attachment contract passes across all four
combinations for Unicode byte ranges, best-passage deduplication, retained text
alongside a terminal gap, and provenance after a metadata-only publication.
A separate upgrade regression verifies that old encrypted/oversized acquisitions
are requeued once; subsequent terminal gaps stay terminal after reopening.
These synthetic functional contracts do not replace large-scale recall,
concurrency or submission-to-render measurements.


After the provenance changes, the focused server search/extraction suite passed
24 tests with two explicit model/PostgreSQL skips. The real local E5 hybrid test
passed separately, as did the native four-backend hybrid/provider/authorization
contract, the four-backend attachment contract, frozen-result provenance, the
one-time acquisition upgrade regression and affected-target Clippy with warnings
denied. No frontend behavior changed in this step. The preceding complete server
suite passed 153 tests, and the complete storage suite passed 112 tests; these
preceded the final extraction/provenance additions.


## Literal metadata indexes

Additive search migrations provide an external-content SQLite FTS5 trigram index
for sender, recipients and subject, plus PostgreSQL pg_trgm GIN indexes for sender
and recipients alongside the existing subject index. SQLite uses the first three
normalized Unicode characters only to obtain candidates, then verifies the whole
literal value with `instr`. Short values retain exact substring semantics without
a trigram prefilter. PostgreSQL uses escaped `LIKE` patterns with an explicit
escape character. Percent signs, underscores, exclamation marks, backslashes and
quotes remain literal input. Both lexical and vector candidate queries use these
paths; resident accelerator predicates retain the same normalized substring rule.

The native index behavior is documented in [SQLite FTS5 trigram tokenization](https://www.sqlite.org/fts5.html#the_trigram_tokenizer)
and [PostgreSQL pg_trgm index support](https://www.postgresql.org/docs/current/pgtrgm.html#PGTRGM-INDEX).
Metadata updates maintain postings in the same transaction. Flags-only changes
avoid rewriting unchanged SQLite metadata postings. The SQLite migration initially
populates this metadata index from retained destination fields; a large upgraded
destination can therefore spend time in that search migration while provider
fallback remains available. Migration timing remains part of scale acceptance.


The metadata-index contract passes across all four backend combinations, including
literal wildcard/escape characters, quoted prefixes, normalized Unicode, two-character
queries, negative full-value checks, restart, subject replacement and account
removal. SQLite postings pass FTS integrity checks before and after cleanup.
`EXPLAIN` confirms a usable SQLite virtual-table index and PostgreSQL subject GIN
path; the PostgreSQL check disables sequential scans for the small fixture and
therefore proves availability, not a production planner choice. The focused server
suite passes 24 tests with two explicit skips. Affected-target Clippy passes with
warnings denied. The full-scale latency and migration benchmarks have not completed.

## Release-scale HTTP profiling harness

`release_mailbox_search_scale` is an opt-in release-only fixture. Its defaults are
250,000 synthetic messages across five accounts and 1,000 distinct mixed queries.
It ingests through the application repositories, publishes the durable outbox,
indexes lexical chunks, produces local E5 passages, commits model jobs and source
completion receipts, reconstructs the SQLite accelerator, and serves the real
Axum router over loopback TCP. Query timing includes HTTP and JSON decoding. It
excludes browser rendering and cannot establish submission-to-render acceptance.
The report always records `full_acceptance: false`.

```sh
CARGO_INCREMENTAL=0 cargo test --release -p dakia-server --lib \
  release_mailbox_search_scale -- --ignored --nocapture
```

For a harness smoke check, set `DAKIA_BENCH_MESSAGES=50` and
`DAKIA_BENCH_QUERIES=12`. Such a run has no scale acceptance value. The separate
million-message growth run sets `DAKIA_BENCH_MESSAGES=1000000`.

Application selection uses `DAKIA_BENCH_DATABASE_URL`; search selection uses
`DAKIA_BENCH_SEARCH_DATABASE_URL`. Unset values retain the normal backend defaults.
Set the search override to the literal `sqlite` to select an independent SQLite
file with a PostgreSQL application store. PostgreSQL overrides must name the
isolated `dakia_search_bench` database with empty account and search tables. The
harness refuses populated databases and never resets schemas. Use fresh isolated
fixtures for each configuration so unrelated indexes do not distort results.

The printed `SCALE_ROOT` is retained, including `scale-app`, a synthetic credential
key and `scale-report.json`. Small fixtures intentionally remain available for
inspection. Benchmark passage construction reuses vectors only for byte-identical
passages individually inferred with the pinned model; this fixture-only cache
never wraps query embeddings. Queries include body-only references, attachment-only
references, participant filters, phrases, typos and multilingual words. Degraded
responses fail the correctness check. HTTP timings are reported separately
from rendered latency. Realistic browser
measurements, cold-start characterization, all four full workloads and recall
acceptance remain separate required gates.


The initial release harness smoke passed with 50 messages across five accounts,
12 unique queries, and zero degraded responses. HTTP-plus-decoding p95 was
41.631 ms and initial local model loading took 957.045 ms. These small-fixture
numbers establish harness operation only, not mailbox-scale performance or browser
latency. The full 250,000-message SQLite fixture has started construction; no
full-scale result is available yet. Affected server Clippy passes with warnings
denied. The harness smoke retained its report under the printed fixture root.


## Independent SQLite postings repair

Stop every process using the selected search file before repair. The repair CLI
uses the application's local operator lock and refuses an active local process;
it does not establish quiescence for separate application roots or remote hosts.
Use the selected search SQLite path explicitly. The operation does not need a
SQLite application store, so it also supports PostgreSQL application storage with
SQLite search storage.

```sh
python3 scripts/repair-search-index.py \
  --data-dir /absolute/application/data \
  --search-sqlite /absolute/selected/search/index.sqlite \
  --backup-dir /absolute/new/repair-snapshot \
  --sqlite-cli /absolute/sqlite-3.54.1-or-newer
```

The command validates the destination identity and known FTS definitions before
creating a snapshot. It verifies the snapshot's canonical table fingerprints,
rebuilds `lexical_terms` and `metadata_terms` together, checks external-content FTS
integrity and database integrity before committing, then rechecks canonical state.
It preserves retained text, embeddings, work queues, completion receipts, replay
frontiers and the destination incarnation. It does not reset search progress or
run extraction or inference. Missing or corrupt accelerator snapshots still use
the existing durable-vector reconstruction path, independently of this repair.
Canonical corruption and unknown schemas are refused. PostgreSQL native-index
repair is described below. Complete multi-process repair acceptance remains a
required gate.

Eleven synthetic repair tests pass with SQLite 3.54.1: both destination indexes,
retained text/vectors/progress, raw snapshot verification, rollback of the combined
repair, unknown schema refusal, the CLI with no SQLite application store, live local
operator-lock refusal, and existing legacy-account repair behavior. These do not
establish the full fault matrix or large-file repair performance.


## Native PostgreSQL search index repair

The repair CLI now supports PostgreSQL search storage with either application
backend. Use compatible PostgreSQL client tools and the pinned operator dependency
`psycopg[binary]==3.3.8`. The connection stays in the environment, not command
arguments. An unspecified search URL follows the PostgreSQL application URL.

```sh
uv run --with 'psycopg[binary]==3.3.8' scripts/repair-search-index.py \
  --data-dir /absolute/application/data \
  --backup-dir /absolute/new/search-repair-snapshot \
  --search-postgres \
  --pg-dump /absolute/postgresql/bin/pg_dump \
  --pg-restore /absolute/postgresql/bin/pg_restore
```

Stop application instances first. The command acquires the local operator lock,
database-wide exclusive application leases for PostgreSQL application/search
connections, and non-waiting SHARE locks on all search tables. Equivalent database
URLs reuse a lease. Existing online application leases or active destination
writers cause refusal before backup. New destination writers wait behind the
repair transaction. Native index rebuilds use a five-second lock timeout and no
work-duration or aggregate size cap.

Before replacing indexes, the tool exports a consistent snapshot and creates a
native custom-format archive of the search schema. It checks every table's archive
catalog entry, hashes and fsyncs the archive, and records canonical table/sequence
fingerprints in a private manifest. Canonical fingerprints stream binary COPY data
in primary-key order with ordinary index scans disabled. Archive and manifest are
durable before repair begins. Individual B-tree, GIN and HNSW replacements run in
one transaction; a later failure rolls back earlier replacements. Canonical data
and index readiness are checked before commit. No automatic restore, owner-state
mutation, extraction or inference occurs. Credentials and native stderr are omitted
from CLI failures.

This uses native [REINDEX INDEX](https://www.postgresql.org/docs/17/sql-reindex.html)
and [pg_dump snapshots and archives](https://www.postgresql.org/docs/17/app-pgdump.html).
Restore requires pgvector and pg_trgm in the destination. Eight native contracts
pass on disposable PostgreSQL 17: exact archive restoration into a separate database,
retained text/vectors/jobs/progress/sequences, atomic rollback, live application
lease refusal, writer refusal, equivalent connection URLs, CLI behavior for both
application backends and failure credential redaction. Eleven SQLite repair
regressions also pass. These tests do not establish every physical-corruption,
disk-exhaustion, multi-host or large-file recovery case; those remain release gates.


## Packaged repair architecture checks

`deploy/repair.Dockerfile` builds a separate operator image with checksum-pinned
SQLite 3.54.1 CLI sources, pinned PostgreSQL 17 base image and pinned Psycopg Python
dependencies. The runtime bundle staging list includes the operator Dockerfile,
repair tools, source fetcher, dependency pins, package checker and
`SEARCH_REPAIR.md`. The operator Dockerfile accepts either source-checkout or
extracted-bundle paths. The full Linux application bundle has not been rebuilt in
this local macOS step.

The image build executes the copied CLI against synthetic independent search data,
checks its snapshot and canonical state, and verifies client versions. An offline
runtime check uses a read-only root and temporary scratch space. Full SQLite repair
contracts can point `DAKIA_TEST_REPAIR_TOOLS_DIR` at the installed image tools while
reading migration fixtures from a read-only source mount. Fixture initialization
uses the pinned CLI; deliberate corruption alters raw shadow bytes separately from
the CLI's defensive-mode path.

Local arm64 and amd64 image builds pass. Both images pass the copied-tool offline
check and all eleven SQLite repair contracts. Both PostgreSQL suites pass with CLI
operations executing the installed image tools; archives created by each image are
restored into a separate database and compared against canonical fingerprints.
The amd64 image was executed through emulation on this arm64 host. The container
workflow now defines native amd64/arm64 operator builds and offline repair tests,
and its multi-platform manifest depends on that gate. Hosted workflow execution
and the remaining complete application/model packaging gates are still unverified.
The release-runner unit suite passes twenty tests with native process access.


## Replaying a retained scale corpus

`release_mailbox_search_replay` reopens a completed synthetic fixture without
re-ingesting messages or re-running passage inference. It requires the retained
fixture key, original report (or an interrupted-run diagnostic), synthetic owner session, five fixture accounts and
completed content counters. Backend selections must match the original report.
Use the original database URLs for PostgreSQL configurations. It waits for local
accelerator recovery and the query model, confirms the immutable synthetic scan
when due, then runs the real HTTP workload. The default mixed workload includes
24 query families: rare references, common words/phrases, participant/subject-only
filters, attachment-only text, multilingual semantic questions, typos and strict
flag/date combinations. `DAKIA_BENCH_WORKLOAD=references` retains the earlier narrow
six-family diagnostic; it cannot establish mixed-workload acceptance.

```sh
DAKIA_BENCH_REUSE_ROOT=/absolute/retained/SCALE_ROOT \
CARGO_INCREMENTAL=0 cargo test --release -p dakia-server --lib \
  release_mailbox_search_replay -- --ignored --nocapture
```

A unique `scale-replay-UUID.json` preserves each report without overwriting the
original. Reports include every query's duration, result count, pending flag and
degradation codes, first HTTP timing and replay startup time. Each replay starts a
new query-model actor and explicitly disables its query-vector cache. Repeated
natural-language queries therefore execute the pinned CPU model too. Reports
identify their workload and cache policy. Filter-only responses need no semantic
metadata, but every account must still report complete coverage. Degradation fails
after the diagnostic report is persisted; latency is reported without a pass/fail
threshold. These HTTP reports still exclude rendering.

For browser work, build `apps/web/dist`, set `DAKIA_BENCH_WEB_ROOT` to its absolute
path and optionally `DAKIA_BENCH_HOLD_SECONDS` to keep the printed loopback server
alive after profiling. The fixture serves that production build through the same
router. No browser-rendering acceptance follows from exposing this endpoint.
Always let an existing corpus-construction process finish before replaying it;
a report file alone does not establish that the original process has stopped.

The retained 50-message smoke corpus replay passed twelve HTTP queries with zero
degradation, p95 36.825 ms. This establishes replay operation only. Server Clippy
passes with warnings denied, and the production web build passes. The full
250,000-message SQLite fixture finished one million embedding turns and accelerator
snapshots. It retains 950,750,000 bytes of body/attachment text across five accounts.
Its first query run exposed repeated degradation and slow fallback, and was stopped
after preserving a diagnostic, the complete corpus and accelerator snapshots. The
diagnostic is not a completed 1,000-query latency report. No full-scale latency gate
has passed.


## Explicit provider-search refresh

Submitting the same search again or using manual refresh now starts a fresh
provider request generation after reconciling the primary mailbox query. The
client takes the refreshed primary query's provider roots, cancels obsolete
provider work and discards its previous pagination chain. A separate retry action
still retries failed accounts without refetching completed accounts. Background
index refreshes continue to preserve in-flight provider roots. A scope-lifetime
guard prevents a late refresh from resetting another search, including leaving
and then returning to the same query.

Mail changes advance an account-specific provider generation after the primary
query settles. A completed account adopts its newest provider root; in-flight
work finishes before catching up once to the latest generation. Other accounts
keep their results and pagination. Previous positive rows remain visible until
the fresh head makes progress. Index-only events and initial stream setup do
not restart provider work. Enter and manual refresh still cancel obsolete work
immediately. Leaving and returning to a query starts fresh provider requests.

The web suite passes 242 tests. Added hook regressions cover completed heads,
obsolete pagination, cancellation/late responses, account events and scope reuse. Four Google
Chrome keyboard journeys pass in light/dark themes at 1480x820 and 390x568,
including same-query Enter, newly arrived provider results, focus, selection and
reload. Rendered screenshots were inspected. An isolated agent-browser Chrome
session also exercised repeat submission, result-region Tab navigation and
horizontal overflow with disposable fixture responses. These fixture checks do
not establish real-provider latency or the large-corpus rendering target.

## Bounded provisional source previews

The scale run exposed an unbounded source-header scan after indexed work timed
out. Provisional cached previews now inspect at most the latest 1,024 headers per
account, concurrently, with one shared 20 ms deadline. SQLite uses the existing
received-order index and a materialized candidate prefix; a fixed outer join order
prevents expansion into a whole-mailbox eligibility scan. PostgreSQL scopes the
candidate prefix by account before applying its limit. Strict current membership,
exclusion, epoch and filter checks still apply. This preview budget does not limit
index size, authoritative indexed lookup or provider search. Missing preview
positives never establish complete coverage.

All three server preview regressions pass, including 50,000 headers and an oldest
message that the preview omits but authoritative selected search finds. The shared
native account contract passes on SQLite and PostgreSQL with bounded-preview
ownership, filter, alternate Trash membership and snooze checks. These regressions
do not establish full-corpus latency or resilience under every failure mode.

A twelve-query release HTTP replay of the retained 250,000-message SQLite corpus
after this change reported p95 194.264 ms, but all twelve responses were degraded
and contained no indexed rows. Its coverage reads took 3.3 to 4.9 ms per account
and accelerator/model startup took 59.65 seconds. This is a failed search-quality
gate, not hybrid-search acceptance. Static account-stage diagnostics and persisted
per-response coverage are available for the next investigation. The 1,000-search,
rendering and other backend-combination gates remain open.

## Candidate and ranking query plans

Scale diagnostics found that SQLite sometimes preferred an account index when
resolving a small list of vector IDs. Candidate identity, payload and exact-cohort
vector fetches now retain integer-primary-key access explicitly. Native query-plan
regressions cover those actual statements. Filter/restart and attachment-provenance
contracts continue to pass on SQLite.

The isolated `release_mailbox_search_components` diagnostic reopens the same
retained fixture and records one account's embedding, spelling, lexical and
semantic durations separately. It does not measure HTTP or rendering and cannot
satisfy latency acceptance. Before the ranking join correction, `reference0000`
took 1,069.722 ms for lexical lookup, 117.587 ms for semantic lookup and 3.029 ms
for its uncached query embedding. The lexical query returned 50 current messages.

SQLite scoring and metadata joins now start with matched identities rather than
expanding through all messages in an account. An opt-in query-plan regression
explains the actual generated statement and requires point lookups for both
metadata joins. A participant cohort filters documents before expanding chunks;
bounded trigram intersections select literal candidates without costly native
phrase-position work. Full literal checks still enforce filter semantics. Lexical
matches already bounded by text evidence verify participant filters directly.
These changes do not establish the required latency gate; release replays remain
necessary, including broad queries and all backend combinations.

SQLite semantic lookup now starts with 256 ANN neighbors and expands through 512
to the existing 1,024 ceiling when needed for 200 distinct messages. A short
approximate page alone does not stop expansion. Unrestricted graphs with at most
4,096 vectors use bounded native exact retrieval. Regressions include a long
message with 700 passages and a repeated-identical-vector case; both retain other
matching messages. Four native vector contracts pass, including replay, snapshot
loss/corruption and current-source checks. Large degenerate clusters still need
the planned scale/recall characterization. Storage/server Clippy passes with
warnings denied.

The subsequent three-query release diagnostic reported HTTP p95 189.740 ms and
two degraded responses. Its attachment query returned a complete hybrid page in
131.953 ms; the initial reference query and participant-filtered query still
exceeded the indexed-work deadline. Static diagnostics distinguish completed
lexical work from unfinished semantic work. This is useful component evidence,
not a passed latency gate. No 1,000-query or rendering acceptance is established.

## Compact SQLite filter-cohort reads

Search migration 0015 adds covering indexes for document filter metadata and
chunk identity/version keys. Participant-cohort discovery uses these indexes,
retains the full literal predicate and source/content-version checks, and keeps
the existing 4,096-vector exact-work envelope. This changes the read path, not
filter meaning, index capacity or candidate recall. A native query-plan contract
requires both covering paths without document/chunk payload scans. The shared
SQLite filter/restart contracts and all five local accelerator contracts pass.

A native CLI diagnostic over a disposable copy of the retained large corpus
returned the same 3,128 chunk IDs in approximately 10 ms with the covering paths,
compared with 987 ms through full document/chunk rows. This isolated measurement
excludes query embeddings, exact cosine work, HTTP, source validation and rendering.
The preceding twelve-query HTTP replay had five degraded responses and p95
182.944 ms, so it did not pass hybrid acceptance. Full release replays are still
required after this migration, with all four backend configurations and rendering.

Retained benchmark replay also validates the isolated PostgreSQL database name
before opening or migrating application/search repositories. Component diagnostics
now separate literal-plan selection from cohort-ID fetching and strict semantic
phrase verification.

The migration replay completed twelve real HTTP queries with p95 170.940 ms;
eight responses had complete coverage and four remained degraded. The initial
reference query was complete at 164.282 ms. Participant traces separated 34 ms
of selector planning from roughly 115 ms of cohort-ID fetching. This remains a
failed acceptance run despite its sub-200 ms response p95.

Bounded cohort IDs now cross the SQLite worker boundary as one JSON row. When a
caught-up lossless F32 accelerator has every cohort key, exact cosine work reads
those same vectors locally, fenced by durable replay checks before and after.
Missing, unavailable or stale accelerators retain the authoritative SQLite vector
path. All 4,096 eligible cohort vectors still participate before message-level
deduplication. A regression compares SQL-only and resident results, including
identical ordering/distances and byte-identical vectors; it also checks missing
keys, another account and the work ceiling. Further release replay is required.

After the resident-cohort change, seven native SQLite hybrid regressions pass,
including actual pinned E5 inference, multilingual/typo queries, phrase constraints,
live exclusions, model failure, frozen pagination and bounded provider handoff
from a stalled database socket. Storage/server Clippy passes with warnings denied.
All eleven native SQLite offline-repair tests also pass against migration 0015.
The PostgreSQL combination tests were not repeated in this check; previous parity
evidence does not replace the remaining release-scale/configuration gates.

The next twelve-query resident-cohort HTTP diagnostic remained degraded on four
queries (p95 174.280 ms, startup 47 seconds). Participant ID discovery still consumed
66 to 122 ms, while completed resident-vector reads took approximately 4 to 23 ms
and exact cosine work about 1.3 ms. This locates a remaining bottleneck; it is not
latency acceptance. Phrase queries also remained degraded.

Migration 0016 changes the compact document index to `(account_id,rowid,...)` so
global trigram postings probe within the requested account's key range. A native
plan regression requires that account-first point lookup. Strict phrase checks
also resolve their bounded message IDs to rowids before checking current versions
and state through compact metadata. SQLite deep/cross-field phrase and shared
filter/restart contracts pass after this change. Native cold-I/O timings vary;
no isolated query-plan measurement substitutes for the release workload.

The account-first release sample completed twelve HTTP requests with p95
176.432 ms and four degraded responses. Startup, including migration and snapshot
rehydration, was 59.032 seconds. A later participant-filtered query completed at
164.046 ms, but the first participant query and both phrase queries still degraded;
the initial reference request also degraded. The persisted diagnostic is
`scale-replay-9079393f-344d-4641-9c02-8c5e456f009a.json` in the retained fixture.
This run fails acceptance. Participant discovery still took 50 to 124 ms, and
strict phrase verification consumed 30 to 67 ms after source validation. Seven
native hybrid regressions, eleven repair tests and Clippy pass at this checkpoint.
No 1,000-query, rendered-latency or full four-configuration gate has passed.

## Independent query lanes and filter-only pages

Search storage now has independent ranking, semantic, phrase-verification and
filter-only read pools, each bounded to the shared eight-account concurrency
limit, plus the existing four-connection control/worker pool (up to thirty-six
search connections total). Application storage
has its separate connection budget. PostgreSQL lanes preserve the configured
schema and connection options. SQLite lanes share the selected database file.
Saturating every ranking slot does not occupy semantic, phrase, filter-only or
control capacity; native isolation contracts pass on both engines.

Read budgets stop expensive work inside the database. SQLite uses a scoped
progress handler signaled on cancellation and cleared before pool reuse;
PostgreSQL uses a transaction-local statement timeout. Deadline/cancellation tests
verify subsequent connection reuse, including SQLite writes and PostgreSQL work
longer than the previous read budget. Ranking, phrase and SQL-cohort reads use
these budgets; normal background indexing retains its own work turns.

The SQLite accelerator also rebuilds indexes over distinct normalized sender,
recipient and subject values during metadata rehydration. Small filtered cohorts
retain exact cosine ranking and durable before/after replay checks. Application
state and phrase verification overlap, and both must succeed. Semantic payload
reads use compact metadata and load attachment details only for attachment hits.
Strict phrase and live Trash checks still apply before returning messages.

Migration 0017 adds a date-ordered metadata index. Filter-only SQLite pages use
complete literal predicates directly on that index, without global substring
posting intersections. This retains date order, keyset pagination and full-mailbox
coverage. A native test verifies correct positive and negative metadata searches
after removing the optional substring accelerator. All five shared filter,
attachment and restart contracts pass across the supported backend combinations;
the eight native server hybrid contracts pass serially. One parallel hybrid run
reported an incomplete page in the backend-combination contract; its isolated and
serial runs passed. The failure artifact is retained. Clippy and all eleven
SQLite repair tests pass at this checkpoint.

The broadened release HTTP diagnostic uses 24 query families and disables query
embedding caching. Its first run on 250,000 messages/950,750,000 text bytes had
fifteen degraded responses. After lane separation and migration 0017, thirteen
responses completed without degradation and eleven remained degraded. Participant,
subject, recipient and no-match filter-only requests completed in 12.55 to 33.93 ms.
Multilingual semantic requests completed in 50.84 to 83.51 ms. Broad words and
phrases still exceeded the lexical deadline; several phrase/flag combinations also
exceeded semantic work deadlines. Available semantic rows now survive many of
these failures. The latest report is
`scale-replay-f2f2843a-b025-442c-b2d2-c2afbf1ee3b6.json` in the retained fixture:
HTTP p95 187.926 ms, first HTTP 201.970 ms, startup 60.398 seconds. Rendering is
excluded. This is a failed acceptance run. The 1,000-query rendered-latency gates
for every database configuration, larger growth characterization and remaining
fault/recovery gates are still open.

## Native lexical projection revision 4

Search migrations SQLite 0018 and PostgreSQL 0013 retain canonical chunks and
embeddings while rebuilding only lexical retrieval. Each canonical word has a
lossless ASCII representation in `native_text`, alongside an account scope
lexeme. This keeps database tokenization from folding meaningful Unicode
characters and scopes posting intersections before fetching message rows.
Canonical text remains available for extraction, repair and phrase verification.

SQLite uses native FTS5 positions for phrases. PostgreSQL uses native positions
only when `phrase_safe` proves that every word has at most 256 positions and the
last position, including the scope lexeme, is at most 16,383. Other chunks use
canonical phrase verification, preserving late passages and repeated-word
phrases. These bounds follow PostgreSQL's [documented text-search
limits](https://www.postgresql.org/docs/17/textsearch-limitations.html).

Shared field, exact-identity, whole-query phrase and spelling-variant weights
remain unchanged. Broad retrieval groups native hits without fetching body text.
Small posting cohorts may still drive canonical checks over matching identities;
bounded probes select the plan and never establish absence or truncate results.
SQLite covering indexes keep chunk identity, document version and score reads
separate from message body payloads. Flag/date vector cohorts likewise use the
existing compact metadata index before expanding passages.

The migration invalidates older lexical processing proofs. Resumable work
revisits retained text, commits the projection and continuation together, and
credits source progress only after accepting the new versioned receipt. Small
documents reuse unchanged chunk identities and spelling references. Installations
predating the spelling projection rebuild missing references during this update.
Embeddings do not need to be regenerated for this lexical revision.

The SQLite repair command recognizes both the earlier external-content layout
and the revision 4 `lexical_native_content` view. It verifies the view definition
before taking a backup and rebuilds only current populated native chunks. A
partial upgrade cannot accidentally index older canonical chunks as native text.
Thirteen native SQLite repair contracts cover this layout and partial-upgrade
repair. Shared long-passage, strict phrase, rollback and restart contracts pass
all four application/search combinations; dictionary upgrades and typo retrieval
pass both native search databases. These are correctness checks, not the release
latency gate.

For an existing synthetic scale fixture, the ignored release helper
`release_mailbox_search_upgrade` advances lexical work and source completion
receipts without calling passage inference. It writes `scale-lexical-upgrade.json`
and refuses non-synthetic accounts or non-benchmark PostgreSQL database names.
Clone a stopped fixture, including WAL files and accelerator snapshots, before
using this diagnostic helper. The original retained fixture remains the baseline.

Ordinary retained messages now advance in batches of at most eight within one
transaction. Each is still limited to one 64 KiB raw document; larger messages
continue one bounded input window per turn. A later failure rolls back earlier
messages, discovery and the continuation in that transaction. This reduces
per-message durable flushes without changing the restart boundary or allowing an
unbounded work turn.

Indexing status no longer hides new model or publication failures merely because
previous completion receipts already cover every eligible message. Such failures
report an error until recovery. Terminal content gaps still allow completion and
remain in the accessible gap details. The endpoint regression exercises completed
processing, model loss and recovery, publication failure and recovery.

Profiling the retained upgrade also found that a new covering index could make
SQLite start job selection from all account documents, then sort the pending
queue. The worker now explicitly selects the history or arrival queue index
first and resolves current document identity/version through its covering index.
The native query-plan regression covers both lanes and rejects a temporary sort
or account document scan. A separate ten-sample CLI diagnostic on the partial
250,000-message fixture measured medians of 38.453 ms for the former selection and
12.185 ms for queue-first selection, including a fresh CLI process each time.
This is a work-queue diagnostic, not search latency acceptance.

The revision 4 mixed diagnostic on the completed 250,000-message/five-account
fixture still fails acceptance: 24 uncached-embedding HTTP searches had p95
189.636 ms, 12 degraded responses, first HTTP 208.964 ms, and initialization
41,595.705 ms. No rendering was measured. A deadline-limited preview is not a
successful hybrid result, so this does not clear rollout.

A follow-up native profile isolated repeated SQLite FTS phrase-cursor setup for
bounded semantic candidates: one 200-message phrase check took 135.837 ms in a
fresh CLI process. Canonical point checks on the same current chunks took
10.824–11.203 ms after the first 35.147 ms read. SQLite semantic phrase constraints
therefore use the bounded canonical path again, with explicit message/version
index access. PostgreSQL retains its safe native-position path and canonical
fallback. Broad lexical retrieval still uses native revision 4 FTS. Shared late
phrase and restart tests pass after this change; the full mixed diagnostic has
not yet been rerun with this follow-up.

SQLite migration 0019 adds compact rank/filter identity access. Both backends now
materialize the ordered, strictly filtered result head before loading message
JSON. Relevance scores, keyset ordering and live-source validation remain the
same. The exact generated SQLite plan must show this materialization and indexed
identity lookups. Shared lexical/restart contracts pass all four combinations;
eleven delivery/ranking contracts pass both search engines, eight integrated
hybrid contracts pass, and thirteen SQLite repair contracts include migration
0019. Storage/server Clippy passes with warnings denied.

The subsequent 24-query release diagnostic still fails: HTTP p95 185.549 ms,
twelve degraded responses, first response 139.256 ms, initialization 57,682.507 ms.
A metadata-only negative query returned provider errors, so `pending=false` alone
does not indicate success. New static-label debug timers distinguish lexical
connection wait, SQL, decoding and live-source validation. No complete rendering
or 1,000-query release latency gate has passed.

Completed search coverage now follows durable folder scan completion and the
current provider epoch, independently of the folder-list refresh timestamp.
The shared native account contract checks old completed generations, account
ownership, epoch changes and missing epoch state, partial scans, continuations,
and resets. Progress discovery also rejects a previous epoch's terminal scan.
The server hybrid fixture deliberately uses an old discovery timestamp so all
four database combinations exercise offline complete-index searches.

The bundled SQLite build now leaves the optional global memory-management LRU
disabled. SQLite otherwise forces every connection's page cache through the
same process-wide LRU mutex, even across independent account and search pools.
Ordinary private page caches retain their existing bounds. A native five-lane
read-only diagnostic over the coherent 250,000-message search copy returned
identical 513-row ordered heads per account. Its repeated p95 SQL timings were
439–441 ms with the global LRU and 90–92 ms without it. Disabling allocation
statistics or enlarging the page cache did not provide the same improvement.
These component timings exclude source validation, embeddings, HTTP and
rendering and do not establish the release latency target.

The subsequent release HTTP diagnostic still failed: 24 mixed searches over
250,000 messages, p95 170.363 ms, nine degraded responses, first response
175.617 ms, initialization 52,888.469 ms. A narrow phrase-plus-reference query
and a negative subject-plus-text query now completed locally, but broad lexical
SQL still reached its 140 ms native budget. Pure semantic queries completed in
47–59 ms. The response timing excludes rendering; returning partial positive
rows within 200 ms does not satisfy the complete hybrid-search requirement.

## Proven lexical score heads

Search migrations SQLite 0020 and PostgreSQL 0014 add compact field lookup and
exact-subject lookup indexes. They retain native lexical revision 4, existing
text, vectors and stage receipts. PostgreSQL narrows exact subjects with an MD5
index and verifies the complete subject, so long headers never enter a B-tree
key and hash collisions cannot change equality.

For an initial search with one lexical unit, the ranker can prove
a global score ceiling from native postings. It uses this path only for a fully
swept lexical account with no pending lexical jobs, no possible exact-subject
bonus, and no matching participant field. A first-chunk count proves that a
highest-scoring field is sufficiently populated; repeated chunks in one long
message cannot establish that density.

The ranker then walks current messages in date order and checks the highest
scoring fields directly. It returns this head only when every requested row
reaches the global ceiling. Every unseen row can then only tie that score and
has an older date or identity. Strict filters use the same predicates and time
boundary as the full ranker before admitting any row. An account-wide ceiling
also bounds that filtered subset. If its highest-scoring stratum cannot fill the
requested head, the full ranker fills the page with other eligible scores.
Short, sparse, participant, exact-subject
and unproven heads continue through the full ranker. These eligibility rules do
not limit indexed storage or truncate general candidate discovery.

The bounds and head share one read snapshot. SQLite uses a cancellable owned
read transaction with the existing 140 ms native budget; PostgreSQL uses a
read-only repeatable-read transaction. Cancellation rolls back the transaction
before its connection is reused. Native contracts on both databases compare
optimized and complete rankings for words, typos, phrases and attachment text,
including older subject matches ahead of newer body-only matches. Additional
contracts prove snapshot stability during concurrent writes and recovery after
cancelled reads. SQLite checks the actual date and field index plan without a
mailbox sort. The existing integrated hybrid contracts pass all four backend
combinations. Release-scale latency remains a separate acceptance gate.

SQLite migration 0021 adds a subject-only covering index for this proof. Small
canonical subject keys stay separate from body and attachment overflow records.
Other fields retain their existing access paths. This is an additive index;
retained vectors and lexical completion receipts remain valid. The current
1,000-query release HTTP diagnostic still fails: 232 degraded responses and
numeric p95 173.407 ms over 250,000 messages across five accounts. Every query
uses an uncached local embedding. The first response took 833.202 ms and startup
56,255.822 ms; cold starts remain separately reported. Rendering is excluded, and
no full four-configuration submission-to-visible-results gate has passed.

## Independent coverage reads

Coverage now runs alongside retrieval rather than gating it. Completed lexical
and semantic positives survive a slow or failed coverage read, with source
authorization and exclusions still checked before presentation. Missing coverage
keeps the account incomplete and supplies a provider continuation. Scan-proof
errors also remain local to coverage. The account and native read deadlines are
unchanged.

A deterministic contract exhausts the actual search control pool while leaving
retrieval lanes healthy. It verifies contributions from both ranked lists,
semantic excerpts and provider fallback, then complete recovery after releasing
the held connections. This contract runs across all four application/search
backend combinations. The fault-injection lease is compiled only with the test
support feature and exposes no database connection types.

A metadata substring index may also decline an unpromising filtered-head plan.
This is never an absence proof: the full ranker still answers the query. A native
contract clears those secondary postings, verifies that filtered lexical results
remain complete, then rebuilds them and verifies that the fast path resumes.

The current repair packages have also been checked against all SQLite search
migrations through 0021 and PostgreSQL through 0014. Their copied tools match the
source hashes, and pinned-client checks plus 13 SQLite and eight PostgreSQL
repair, rollback, lease, credential-redaction and archive-restore contracts pass.
Linux ARM64 ran natively; Linux AMD64 ran under emulation. This establishes repair
package behavior on those configurations, not native Intel Mac or model/latency
acceptance.


## Field-specific lexical projection revision 5

SQLite search migration 0022 and PostgreSQL migration 0015 add a field identity
to every encoded word. Account scoping, canonical Unicode text, phrase positions
and shared ranking weights stay the same. The bound estimator can probe each
field directly and stop after enough postings to choose a plan. These bounded
samples never establish coverage, truncate candidates or relax strict filters;
unproven heads still use the complete ranker.

Migration retains revision 4 postings while the resumable worker replaces them
message by message. Both formats remain searchable during backfill, including
later passages in a partially processed large message. Only current-version
receipts establish completion. Existing embedding identities and bytes remain
unchanged. Native SQLite and PostgreSQL upgrade contracts interrupt a long body,
reopen the store, finish processing and check these guarantees.

The repair command recognizes the exact revision 5 external-content view and
rebuilds both populated formats during an interrupted upgrade. Thirteen SQLite
repair contracts pass with migration 0022. The rebuilt repair tools match current
source hashes and pass these contracts plus eight PostgreSQL repair contracts
through migration 0015 on native Linux ARM64 and emulated Linux AMD64. Release-scale
HTTP measurements are still pending; the previous revision 4 results above do not
establish revision 5 acceptance.


## Multi-unit score ceilings

The ranked-head proof now bounds each query unit independently and adds the
whole-query phrase ceiling. Each admitted message must reach every bound, with
all strict filters applied before the date-ordered head. Units can match different
fields or chunks; a phrase must remain contiguous in one chunk. Exact-subject and
possible participant bonuses continue through the complete ranker. Sparse or
unproven intersections also fall back without truncating discovery.

Native comparisons cover mixed subject/body units, multiple words, typo variants,
attachment text, exact identities, missing words and sparse flag intersections.
SQLite passes the focused comparison. PostgreSQL passes it with fresh accounts
in four runs after changing folder membership to evaluate its requested folder
array once. This preserves literal folder names and any-membership behavior.
The former set-returning membership predicate repeatedly timed out on a newly
seeded account; diagnostic plans showed underestimated account cardinality and
repeated folder-list evaluation. Production deadlines are unchanged.

The parallel full storage run has one interrupted bound comparison while a
separate large-corpus backfill runs; the other 94 tests pass. This remains an
unresolved reliability observation, not a ranking correctness or latency pass.
The revision 5 release fixture is still being upgraded, and no new release-scale
submission-to-visible-results gate has passed.


## Canonical metadata hints and bounded header previews

The latest 1,000-query release HTTP run has 43 degraded responses, p95 159.246 ms,
first response 117.764 ms and startup 29,759.150 ms. All invoice and standalone typo
queries complete. Forty-one failures are sender-filtered phrases; two are narrow
multi-unit queries. Rendering is excluded, so this remains a failed acceptance run.

SQLite migration 0023 adds covering indexes over canonical sender and recipient
values. The existing subject index serves the same purpose. Planning hints now
read these canonical values rather than intersecting secondary substring postings.
The complete date-ordered proof and strict predicates still admit every returned
row. Missing optional postings no longer disable that proof. A five-account native
probe returned identical hint answers with p95 1.970 ms versus 85.915 ms for the
former hint. This is a component diagnostic, not submission-to-render acceptance.
Rare text anchors also bypass dense-head estimation, and identical unit/phrase
predicates share one check while retaining both score contributions.

Provisional header previews keep their 1,024-message window. Both databases now
find its oldest date/identity boundary through the existing order index and read
only that inclusive range. This avoids copying every header into a temporary
relation before filtering and sorting. The SQLite 50,000-header fixture measured
about 2.5 ms for the positive preview read, compared with the former read crossing
its unchanged 20 ms deadline. Full indexed and provider searches remain unbounded.
Broader checks and release measurements for these changes are pending.


Metadata-only queries also use small canonical cohorts when a literal filter is
selective. SQLite probes at most 1,025 matching integer identities through the
canonical field index. Fewer matches establish the entire cohort; only that
complete cohort, containing at most 1,024 messages, may use a bounded date sort.
Larger cohorts retain the original unbounded date-ordered plan. Probe and result
read share one snapshot and the existing read deadline. Every strict predicate,
keyset and live application-state validation still applies. PostgreSQL keeps its
native planner.

This fixes a cold oldest-message read in the 50,000-header server fixture. The
retained failed fixture contained the correct message in both stores; replaying
its former query took 349 ms cold and about 7 ms warm. The revised full journey
passes without tracing: 7.09 ms for the provisional preview and 80.15 ms for the
oldest indexed result. Shared strict-filter, attachment, restart and missing-index
contracts pass across all four database combinations. These measurements do not
replace the release submission-to-visible-results gate.

### Filter cohorts and source validation batches

SQLite can use a complete canonical metadata cohort to drive text ranking as
well as metadata-only searches. A probe reads at most 1,025 identities. Only a
cohort with fewer than 1,025 identities is accepted, and the complete original
ranker checks every text unit, phrase, source version and strict filter within
the same snapshot. Broader cohorts retain unbounded native discovery. This
threshold bounds one work plan, not mailbox coverage or index storage.

Source validation transfers each existing bounded 200-message page from SQLite
as one JSON batch. Its transaction, ownership, eligibility, epoch, pending
outbox and snooze checks are unchanged. The application contract verifies all
200 Unicode payloads, empty input and exclusion of an update awaiting delivery.
The disposable SQLite vector metadata index also tracks unread, starred and
internal dates. Small intersecting cohorts use exact vectors; broad cohorts use
filtered ANN, with the existing durable replay checks before and after selection.

The five filter, provenance and replay tests pass against all four database
combinations after these changes. The default parallel storage run passed 95
cases and failed one ranking keyset case on the unchanged 140 ms database
budget; it is not a clean suite result. The preceding 1,000-query release run
returned 143 degraded results at p95 175.015 ms. That run fails acceptance, and
neither the batch changes nor a numeric HTTP percentile establish the required
submission-to-visible-results gate. Release verification remains in progress.
