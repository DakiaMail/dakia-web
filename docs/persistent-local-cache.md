# Persistent local mail cache

This extends phase one's reading-loop work. Reopening Dakia restores saved folder
headers after a live session and account check. Switching previously visited
folders reuses these snapshots immediately while the live first page refreshes.
Bodies are read from disk only when opened or prefetched, without hydrating the
whole mailbox into memory at startup. Current network data replaces saved copies.

The viewport prefetch queue covers every message intersecting the list, including
partially visible rows. Virtualizer overscan does not count. Scrolling replaces
queued work; one speculative body runs at a time. Focus and the next reading target
have priority. A foreground open cancels an unrelated unused speculative request.
Hidden/offline/Save-Data/2G sessions skip speculation. Prefetching never mounts a
reader, loads remote images, marks mail read or discovers conversations.

## Storage and correctness

- IndexedDB has a 64 MiB serialized-mail budget (plus database overhead), at most
  2,000 entries, 32 folder snapshots and 200 headers
  per snapshot, with seven-day retention. Each record is at most 4 MiB. Larger
  messages remain available through the ordinary network reader.
- Records are scoped by authenticated user and account. Fresh account membership
  and identity are checked before hydration. Removed/reconnected identities are
  discarded. Authentication failures, sign-out and account removal clear saved
  mail. Other updated tabs reauthorize after revocation. Existing tabs need Reload to
  use this cache and its revocation handling. A database epoch prevents late
  writes from a previously authorized tab from repopulating cleared storage.
- Sessions, CSRF tokens, credentials, account settings, searches, drafts, operation
  requests, attachments and external-image responses are never persisted here.
  Received HTML remains untrusted and uses the existing sanitizer and iframe.
- Folder snapshots discard opaque provider cursors and completeness claims. Only
  the fresh first page supplies pagination. Membership-changing mail actions invalidate saved
  folder views immediately; stale responses during mutation reconciliation cannot
  save an old view. No mutation is ever replayed from the cache.
- Storage metadata is separate from bodies so eviction and startup avoid reading
  the whole body cache. Serialization/writes are batched after query notifications,
  outside the immediate rendering callback. Cached reads do not renew retention.
  Storage denial, quota failures and corrupt records fall back to online use.
- Up to three recently unused speculative bodies remain in memory in addition to
  current viewport/intent targets. Opened messages keep normal reader retention.
- The service worker precaches only a build-generated allowlist of hashed static
  assets. It never caches navigation, API/auth, mail, images or notification links.
  Activation prunes obsolete assets and preserves unrelated application caches.

A cold offline launch cannot verify the session and does not unlock saved mail.
An already-authorized mounted session can switch saved folders and read cached
bodies offline. Network writes still require the server. This is a local recent-
mail cache, not a full offline mailbox or a persistent outgoing-action queue.
Signing out clears saved mail, but browser storage is not encryption against
someone who already has access to the browser profile or the signed-in origin.

## Verification

Use Node 24 and Google Chrome. Unit coverage uses fake-indexeddb; browser coverage
uses real Chrome IndexedDB and disposable mail. `persistent-mail.spec.ts` holds
server responses until saved rows/bodies render, checks keyboard folder navigation,
authorization, another owner, cross-tab sign-out and mounted offline reading.
`persistent-speed.spec.ts` measures reopen/folder/body latency with 600 ms list and
body delays. `reading-prefetch.spec.ts` checks viewport coverage and overscan,
scroll replacement, no image/action/discovery requests and offline suppression.

```
npm ci
npm test
npm run build
DAKIA_READING_FULL=1 DAKIA_READING_PORT=4202 npm exec --workspace @dakia/web playwright -- test --config playwright.reading.config.ts persistent-mail.spec.ts persistent-speed.spec.ts reading-prefetch.spec.ts
DAKIA_READING_FULL=1 DAKIA_READING_PORT=4202 npm exec --workspace @dakia/web playwright -- test --config playwright.reading.config.ts --workers=4
```
