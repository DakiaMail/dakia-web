# Mail view loading investigation

Base: `8f5e80cd36529dcf0fa73c9f84762d76414584df` (GitHub main verified before editing).

## What opening a view did

- Needs attention mounts `AttentionStrip`, which requests `folder:INBOX` with the attention filter. The server reads local metadata, but the date-ordered inbox index still walks unrelated messages looking for sparse attention matches. The component hides itself while no results have arrived.
- Starred requests `is:starred`. It already uses the local projection and background sync supplies provider coverage. Its date index contains all messages, so a sparse starred view can walk the entire mailbox. It does not need a new provider fetch policy.
- Snoozed requests `is:snoozed`. Unlike Starred, it falls through to general search, potentially returning a provisional indexed/cache page followed by provider folder discovery, scans, per-row snooze checks and continuation requests. Local snoozes already determine membership. Empty results can still require provider work.
- The snooze count joins message JSON to local snoozes without a usable thread expression index, affecting both the sidebar and client filtering of the Snoozed list.

## Change

Add partial date indexes for starred messages and attention candidates. Keep the exact attention predicate for expiry and manual dismissal after candidate selection. SQLite maintains these indexes when flags and classifications change.

Route the exact Snoozed navigation query through the existing local keyset pagination machinery. Start from local snoozes and find message rows using a thread index. Its local coverage can be complete independently of provider connectivity. Other search queries keep their existing search path. Use the same thread index for the snooze count.

The thread expression is explicitly cast to TEXT on both the index and queries. Without that cast SQLite did not use the expression index when comparing it with the TEXT thread key. CROSS JOIN keeps the small snooze table on the outer side.

## Synthetic query evidence

Run `python3 docs/performance/fast-mail-views/benchmark.py`. It extracts the actual SQL from the storage source, applies the account migrations, and loads 100,000 fictional messages with 10 stars, 10 attention items and 10 snoozes. Measurements are median of three in-memory SQLite runs on the development Mac, without HTTP, rendering or a live mailbox. Full query plans are in `query-plans.txt`.

| Query | Before | After |
| --- | ---: | ---: |
| Starred | 162.90 ms | 0.02 ms |
| Needs attention | 409.48 ms | 0.05 ms |
| New local Snoozed lookup | 1,499.82 ms without thread index | 0.03 ms |

The Snoozed before column measures the new local query without its index, not the old provider search. No end-to-end speedup or live-provider latency is claimed. The old provider round trips are eliminated from this navigation path.

Regression tests cover local empty/offline and paginated Snoozed navigation, tied timestamps, expired snoozes, obsolete provider references, and index selection. Existing attention tests cover expiry, manual marking and dismissal. Existing Starred tests cover offline use, multiple accounts, pagination, changes and provider epoch invalidation.

The new indexes are built once when an account database migrates. That first upgrade can take longer for a large mailbox and adds index storage/write cost. No production data or service was changed.

## Validation

- Storage suite: 100 passing tests, 3 manual benchmarks ignored. The added query-plan regression also passes separately.
- Server Starred/Snoozed group: 4 passing tests, including empty/offline Snoozed and a 55-message continuation through the production search handler.
- Frontend: 242 tests pass with `NODE_OPTIONS=--no-experimental-webstorage npm test`. Without that flag the local Node 22.23.2 runtime exposes an unavailable localStorage global to two existing test files.
- `npm run build` passes.
- Synthetic SQL benchmark asserts identical results before and after and checks the three intended indexes are selected.
- Live mailbox, browser rendering latency and production upgrade timing were not tested.
