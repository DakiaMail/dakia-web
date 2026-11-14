# Broader loading audit

The code scan covered the web app and server paths behind startup, Inbox tabs and type filters, attention, saved views, Sent/Trash, search, reader/conversations, drafts/scheduled, account settings, recipient suggestions and live updates. This is a source audit with synthetic tests, not a claim that every screen or live mailbox was timed.

| Path | Finding and action |
| --- | --- |
| Personal / Automated | Local query, but the optional JSON predicate could not seek the existing kind index. Add a direct bound kind predicate and a matching effective-kind/date index. Missing classifications still count as Personal. |
| Automated type filters | Array membership was evaluated across Inbox rows before LIMIT. Add a small derived type projection, kept transactionally current by message insert/update/delete triggers, with an index on type/date/id. Multiple types remain independent and overlapping. Avoid rewriting it when only unrelated flags change. |
| Returning to a tab/filter | React Query already keys each classification scope separately and keeps data fresh for 30 seconds. Chrome regression verifies Personal, Automated, Receipts, Orders and revisits without duplicate requests. Avoid speculative fetching of every filter. |
| Sent / Trash, one account | Bypassed cached search and waited for a provider page. Use the same cache-first search path as multi-account folders. Chrome regression holds provider continuation open while cached rows remain visible. Provider coverage and reconciliation remain explicit. |
| Attention / Starred / Snoozed | Optimizations and evidence are in the parent README. |
| Startup | Session and account authorization precede browser-cache activation. Keep these checks. IndexedDB hydration can add startup latency; no live startup profile was collected. |
| Reader / conversations | Body reads already try browser/server caches, viewport and next-message prefetch are bounded, and foreground requests cancel unrelated speculative work. Conversation snapshot and provider verification have separate lifecycles. Preserve those paths. Cold uncached bodies still require provider I/O. |
| Drafts / Scheduled | Local database reads; account requests run concurrently. A slow account can delay the combined list. No measured mailbox-scale regression was established in this scan. |
| Settings | Settings and folder queries run independently. Folder discovery deliberately contacts the provider to offer current folders. Do not replace current configuration choices with a silently stale list. |
| Recipient suggestions | Search/group over verified recipient evidence. Potential scale hotspot, without a measured failure here. Preserve the outgoing-only evidence boundary. |
| Live updates / actions | Events are scoped and debounced; queries reuse existing data while refreshing. Actions already use optimistic state and reconcile failures. No blanket invalidation or additional polling added. |

## Filter evidence

The expanded Python benchmark extracts the actual Inbox SQL tail, seeds 100,000 synthetic messages with 1,000 automated messages and 10 overlapping receipt/order messages, and checks identical results and intended index selection before/after. One local run measured Automated at 7.62 ms before / 0.46 ms after, and Receipts at 150.65 ms before / 0.28 ms after. These filter timings are individual query runs, not page latency. Full plans are in `expanded-query-plans.txt`.

Storage regressions verify correction/removal, overlapping types, deleting messages, and tied-date keyset pagination. Chrome tests exercise real navigation, matching request scopes, cached revisits, and provisional folder rendering during a blocked provider request.

The additional projection and indexes add storage and some write cost. Account migration builds them once. Cold network reads, remote image loading and external provider outages cannot be made instantaneous by these changes.

## Search plan comparison

The existing finite 50,000-body benchmark exposed a tradeoff. Forcing date order took 9.67 ms for the rare term and 8.47 ms for no matches; allowing direct FTS rowid seeks took 2.60 ms and 0.33 ms. Conversely a common term slowed from 173 ms to 469 ms with unconditional rowid seeks. The final implementation probes at most 512 FTS hits and uses seeks only for sparse results; common terms retain the original date-order path. Search semantics and substring verification are unchanged. The bounded cutoff is a documented tuning choice, not a universal latency guarantee.
