# Privacy

Dakia runs on infrastructure you control. This initial overview describes the current design; a detailed network and retention inventory is planned.

Mail sync and sending connect to the email providers you configure. Classification runs on your server. Translation runs on your server by default; an administrator can choose device-only translation or device translation with host fallback in Workspace. Public model files may be downloaded from their upstream hosts. An optional OIDC provider handles sign-in when configured.

Dakia stores mail caches, drafts, attachments, settings, and operational records locally. Credential encryption does not encrypt all stored mail. Your host administrator and anyone with sufficient host or backup access can read that data.

Separate Dakia logins have separate mailbox connections and mail access. The local owner can invite people, but that permission does not let them read another person's mail through the app. Host administrators still have access to the underlying files and keys.

Remote images are allowed by default. In Workspace, turn on **Block external images by default** to load them only when you choose, or **Block hidden images and tracking pixels** to skip explicitly hidden or tiny images. These preferences are saved for your signed-in user on that browser; set them separately on other devices. Remote images can contact sender-controlled services through Dakia's proxy when allowed. Proxying hides the browser's direct connection but does not eliminate tracking: unique image URLs can still identify a message or recipient.

Recent folder headers and message bodies are also saved on this browser for faster reopening and folder switching, within a 64 MiB serialized-mail budget and seven-day retention. A fresh session and account check is required before saved mail is shown on a cold launch. Signing out, an authentication failure, or account removal clears saved mail; other updated tabs reauthorize. Someone with access to your browser profile may be able to read this cache. Drafts, outgoing actions, credentials, session tokens, attachments and remote-image responses are not saved in this browser mail cache. See [cache limits and behavior](persistent-local-cache.md).

The service worker caches only build-owned static app assets. It does not cache navigation, API/auth responses, mail, remote images or notification links. An already-authorized mounted session can read saved folders and bodies offline; a cold offline launch cannot unlock saved mail. Installing Dakia does not download a full offline mailbox.

Background prefetching covers messages visible in the mail list without marking them read, loading remote images or discovering conversations.

Optional Web Push uses browser push infrastructure. The push payload does not contain sender, subject, or message body text. A signed-in browser can fetch an authorized message summary from Dakia and display the sender, subject, account, and a preview in the notification. If that fetch is unavailable, it shows a generic notification. Notification text may be visible on a shared or locked screen; control notification display through your browser and device settings.

Invitation links contain a single-use secret. Share them privately. The server stores a hash, and the join page keeps the code in the tab's history after removing it from the address bar. It expires after 24 hours or becomes invalid when used or revoked.

Your host and reverse proxy can keep their own logs. Operational diagnostics are retained locally and may contain sensitive metadata; keep diagnostics and backups private. A field-by-field diagnostics review and retention guide are planned.

A remote image policy, provider-side retention, and copies in backups can affect deletion. Removing something from Dakia does not necessarily erase every copy held by your provider or backups.

On-device translation saves verified public language packs in browser Cache Storage. They can remain after sign-out, but message text and translated output are not stored in that pack cache. Clearing site data removes saved packs. See [translation settings](on-device-translation.md).

## Release checks

When a local owner opens Dakia, the server can check GitHub for published releases. Successful checks are cached for 24 hours and failures for one hour. GitHub receives the server's network address and an application User-Agent; requests do not contain mailbox data, credentials, or user identifiers. Browser tabs ask only the Dakia server. Set `DAKIA_RELEASE_CHECKS=false` in the server environment to disable automatic checks. Manual host upgrade checks still contact GitHub. See [upgrading](upgrading.md).
