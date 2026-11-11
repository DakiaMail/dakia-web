# Features

Dakia connects to Microsoft accounts through [Microsoft Graph](microsoft-oauth.md), Gmail through Google sign-in or app passwords, and other existing IMAP/SMTP mailboxes. It does not replace your email provider. See [getting started](getting-started.md) for owner setup, joining, and connecting mail.

## Separate people and mailboxes

The local owner can invite other people with single-use links. Each person chooses their own login and connects their own email accounts. Their unified inbox, search, drafts, and attachments stay scoped to their account. Invitation management does not grant access to someone else's mail.

Each person can connect more than one mailbox. Use the account list for one mailbox or Unified inbox for all of your own accounts.

## Read and organize

Conversations, search, smart categories, stars, snooze, and bulk actions help organize mail. Workspace provides mailbox settings, reconnection, removal, and per-browser image privacy controls.

![Desktop Workspace with a connected fictional mailbox and image privacy controls](images/workspace-desktop.png)

## Block unwanted senders

Open an email and choose **More actions → Block sender** in the toolbar. Future emails from that address move to Trash across your mailboxes without notifications while Dakia syncs. Older mail stays in place unless you review it and confirm cleanup. Manage blocks in **Workspace → Blocked senders**. Restored emails stay restored, and unblocking does not empty or restore Trash.

## Write and send

Rich-text composing, attachments, drafts, replies, forwarding, and scheduled sending. Dakia must remain running and connected for scheduled work.

## On your devices

The web app adapts to desktop and mobile screens and can be installed as a PWA. Sign in on another device with the same Dakia login to access your mailboxes. Recent folders and bodies are cached on this browser for fast reopening after session checks. An already-authorized session can read saved mail offline; installation does not download a full offline mailbox.

<img src="images/mail-mobile.png" alt="Mobile view of a message in Bob's fictional mailbox" width="390">

Both screenshots use fictional mailboxes. They show the running app, not a design mockup.

Optional push notifications require host configuration and browser permission. Notifications can include a summary fetched by the signed-in browser; see [privacy](privacy.md).

## Translate on your device

Choose **Translate to English** while reading an email. The first on-device translation downloads the language pack, with a circular progress indicator and a cancel button. Saved packs survive page reloads and browser restarts, so later translations can reuse them without another download. Switch back to the original email at any time.

In **Workspace → Translation**, administrators can choose device-only translation, device translation with host fallback, or host-only translation. Host fallback lets devices that cannot finish a translation use the server instead. Host-only remains the default. You can also remove saved language packs from your browser in Workspace.

See [translation settings and saved languages](on-device-translation.md) for details, including offline use and browser storage limits.

## Local mail tools

Classification runs on the host. Provider compatibility, resource requirements, and platform coverage are still being tested.
