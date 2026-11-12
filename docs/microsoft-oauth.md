# Microsoft accounts

Dakia connects Outlook.com, Hotmail and Microsoft 365 mailboxes using **Microsoft Graph**, like pl_mail. It does not use IMAP/SMTP OAuth, so it does not require disabling Microsoft 365 Security Defaults or enabling SMTP AUTH. This is mailbox authorization, not signing into Dakia.

Use a build containing Microsoft support; if your pinned image predates it, build this checkout with `docker build -t dakia:local .` and set `DAKIA_IMAGE=dakia:local`.

## Operator setup

1. In Microsoft Entra ID → App registrations, register an application.
2. Choose **Accounts in any organizational directory and personal Microsoft accounts** for both work/school and personal mailboxes (tenant `common`).
3. Add a **Web**, not SPA, redirect URI: `https://your-dakia-address/api/accounts/microsoft/callback`. It must exactly match `DAKIA_ORIGIN` plus that path.
4. Copy the Application (client) ID. Under Certificates & secrets, create a secret and copy its **Value**, not its ID. Note its expiry and replace it before it expires.
5. Add delegated Microsoft Graph permissions **User.Read**, **Mail.ReadWrite**, **Mail.Send**, and the identity permissions **openid**, **email**, **profile**, **offline_access**. No application permissions are needed. Tenants that restrict user consent must grant administrator consent.
6. Set `DAKIA_MICROSOFT_CLIENT_ID` and `DAKIA_MICROSOFT_CLIENT_SECRET`, then recreate the Dakia container. Partial credentials fail startup.

Prefer `DAKIA_MICROSOFT_SECRET_FILE` for the secret: mount a private file read-only, set that environment variable to its container path, and remove the environment secret. Add the mount and environment variable in a private Compose override; the supplied Compose file passes the ID, environment secret, and tenant through.

`DAKIA_MICROSOFT_TENANT` defaults to `common`. Use `organizations` for only work/school accounts, `consumers` for only personal accounts, or a tenant GUID for one organization. The app registration's supported account types must agree. Sovereign-cloud endpoints are not supported.

## Connect and reconnect

In **Connect an account**, select **Microsoft (Outlook / Microsoft 365)**, enter an account name and the mailbox’s primary email or Microsoft sign-in address, then choose **Continue with Microsoft**. Use that same Microsoft account on the consent page. Dakia stores Graph’s actual mailbox SMTP address, not a different work/school login name. No Microsoft password is collected or saved by Dakia. Access and refresh tokens stay on the server; the refresh token is encrypted with the installation credential key. Rotated refresh tokens are saved automatically.

For an expired or revoked grant, choose **Reconnect** in Workspace. Reconnection keeps the account ID and cached mail. The Microsoft user ID must still match the original connection. If Microsoft has changed the primary SMTP address, reconnection updates it without replacing the account or its cached mail. Changing the application ID or tenant requires reconnection; changing only the client secret does not.

If sign-in fails, check the exact redirect URI, secret value/expiry, tenant and account types, and administrator consent. An account created with an external email address may have no Outlook mailbox: open Outlook first and provision it. Disabling Microsoft sign-in does not delete accounts or credentials; removing a mailbox deletes its local connection but does not revoke Microsoft's grant. Revoke the application separately in Microsoft account/organization settings if needed.

## Graph push and polling fallback

For a public HTTPS `DAKIA_ORIGIN`, Dakia automatically registers a content-free Graph mail subscription. Microsoft must be able to POST to **`/api/microsoft/notifications`** without a Dakia login, VPN, or proxy authentication. Forward the query string unchanged: Microsoft’s validation challenge must be echoed as plain text synchronously. Normal notifications require the matching subscription ID and secret client state; untrusted payload contents are never treated as mail data.

Subscriptions persist across restarts, renew before expiry, and handle missed/removed/reauthorization lifecycle notifications. Registration/renewal failures back off and retain polling; installations reachable only over a LAN or VPN continue to work without Graph push. Removing an account attempts remote subscription cleanup after local revocation; an unreachable or unauthorized subscription expires at Microsoft without granting access to the removed account.

These Graph subscriptions are separate from browser Web Push notification preferences. No administration UI or extra Outlook permissions are added.

## Mail support and limits

- Folder discovery (including children), paginated mail sync, headers, read/unread, stars, archive, trash, junk, moves, MIME bodies and attachments, replies, and sending use Graph.
- Sent copies are filed by Microsoft, never appended a second time. Bcc recipients are preserved when sending.
- Initial import uses newest-first Graph delta enumeration, with independent arrival checks while history is still importing. Subsequent rounds fetch only additions, updates and removals. Opaque cursors are durable; expired cursors and changed folder bindings restart enumeration without deleting saved mail. Only completed delta enumeration, never ordinary list/search pages, can authorize snapshot pruning.
- Message metadata, MIME bodies, and bulk mail changes use Graph batches of at most 20 requests. Partial responses retain per-message outcomes; throttled requests back off without replaying uncertain writes. Provider-side text searches may still retrieve MIME bodies; normal search uses Dakia’s local index.
- Sending uses the single-request MIME endpoint with the same **4 MiB raw-MIME limit** as pl_mail (including attachments, MIME encoding overhead and Bcc headers, before Graph base64 encoding). Larger messages fail explicitly; neither this path nor pl_mail implements resumable attachment uploads. Reading retains Dakia’s existing content limits.
- Definitive throttling/service refusals and pre-connection failures are deferred durably with a bounded retry budget, keeping the same MIME and Message-ID. A timeout or lost gateway acknowledgement stays delivery-unknown: a positive, read-only Sent Items lookup can confirm it later, but a negative lookup never causes an automatic resend.
- Immutable IDs are requested on every Graph call and support is probed and recorded. Dakia links returned IDs for its own moves. For provider-side moves without immutable-ID support, it preserves the local identity only when RFC Message-ID and header evidence identify one old candidate and Graph proves its old locator is gone. Live copies and ambiguous matches stay separate rather than being silently merged.
- This adds mail only, not Outlook calendars, contacts, categories, shared mailboxes, OneDrive, or Microsoft login to Dakia.

Registration reference: [pl_mail Microsoft provider](https://github.com/karatektus/pl_mail/blob/main/docs/providers/microsoft.md). Protocol reference: [Microsoft Graph mail API](https://learn.microsoft.com/en-us/graph/api/resources/mail-api-overview).
