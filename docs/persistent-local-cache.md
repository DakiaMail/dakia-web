# Saved mail in your browser

Dakia saves recent folder views and message bodies in this browser so returning to mail can be faster. A fresh startup checks your session and connected accounts before showing saved mail, then refreshes it in the background.

## What is saved

The mail cache uses IndexedDB and is scoped to your Dakia user and connected accounts. It keeps up to 64 MiB, 2,000 entries, and 32 folder snapshots, with entries retained for at most seven days. These are eviction limits, not a promise to download that much mail. Individual cached bodies over 4 MiB aren't restored from this cache.

Visible messages are prefetched serially without marking them read or loading external images. Attachments aren't a complete offline download. Translation language packs use a separate public-file cache and have different retention rules.

## Offline limits

An already-authorized, open session can read mail that is available locally. A fresh offline startup can't perform the required session and account checks. Missing bodies, uncached folders, new search results, and mail changes can still require the server. Installing Dakia as a PWA doesn't download a complete mailbox or provide offline sign-in.

For offline translation, the message must already be available and the required language pack saved. See [translation](on-device-translation.md).

## Clearing saved mail

Signing out, changing users, or removing an account clears saved mail and invalidates other updated tabs. Clearing the site's browser data also removes it. If storage is unavailable, corrupt, or full, Dakia continues to work online without relying on the cache.

Treat browser profiles as private storage. On a shared computer, sign out before another person uses Dakia, or use separate browser profiles. See [privacy](privacy.md).
