# Getting started

[Install Dakia](hosting.md), then open its HTTPS address. You can use one installation by yourself or share it with other people through invitations.

## Create the first login

The first person to set up a new installation becomes its owner. Open Dakia and choose **Sign in**, then choose a username and a password with at least 12 characters.

No setup code or host command is needed. A new installation accepts its first login from whoever reaches it, so create your login immediately after starting Dakia and keep the address private until you have. If your sign-in page still asks for a setup code, the image predates this change: read it with `docker compose exec dakia cat /secrets/setup-token`.

Your Dakia login is separate from your email provider's login. Once owner setup is complete, the sign-in page asks only for your username and password.

## Connect an email account

1. Choose **Connect account** in the welcome screen or Workspace.
2. Give the mailbox a name, such as Personal or Work, and enter its email address.
3. Choose Gmail for Gmail settings, or Automatic for another provider. If automatic discovery cannot find settings, enter the provider's IMAP and SMTP hostnames, ports, username, and encryption settings manually.
4. If the operator has [configured Gmail sign-in](gmail-oauth.md), choose **Continue with Google**, sign in to that same email account, and allow mail access. Otherwise, enter the mailbox password or app password and choose **Connect account**. This is separate from your Dakia password. To use an app password when Google sign-in is available, select **Use an app password**.
5. Dakia saves the account and checks its connection. If the check fails, use the error and your provider's settings to correct the connection and retry.

Use TLS or required STARTTLS as specified by your provider. Your host must be able to reach both the incoming IMAP server and outgoing SMTP server. Dakia uses your existing provider; you do not need to change MX records or run an incoming mail server.

Repeat this for additional mailboxes. **Unified inbox** shows your connected accounts together, and the account list lets you open one mailbox at a time. Workspace provides mailbox settings, reconnection, and removal controls.

## Invite another person

Only the local owner sees invitation controls.

1. Open **Workspace** using the sidebar. On mobile, open navigation first.
2. Find **Invite someone** and choose **Create invitation**.
3. Choose **Copy link**, or select and copy the displayed link. Share it privately with the intended person. Dakia does not email the invitation for you.

Each link works once and expires after 24 hours. There can be up to 20 unused, unexpired invitations. Copy a newly created link before leaving the page: pending invitations show their expiry and a revoke button, but their links cannot be retrieved later. Create a replacement if needed.

An invitation grants permission to create a Dakia login. It does not grant network access. If the service is restricted to your LAN, VPN, or Tailscale network, the recipient needs access to that network too.

## Join an installation

Open the complete invitation link from the owner. The invitation code fills automatically. Choose a unique username and a password with at least 12 characters, then choose **Continue**. You can then connect your own email accounts.

The link's `#` section contains the invitation code. Preserve the complete link when sharing it. The page removes the code from the address bar and retains it in that tab's history so a reload still works. If scripts are unavailable, copy the code after `#` into the Invitation code field yourself.

If a username is already taken, choose another. That failed attempt does not consume the invitation. An expired, revoked, or already-used invitation requires a new link from the owner.

## Keep each person's mail separate

Each person has their own Dakia login, mailbox connections, messages, drafts, attachments, and search results. The unified inbox combines only that person's mailboxes. The owner's ability to invite people does not let them read another person's mail through the app.

The person operating the host still has administrative access to its files and backups. See [security](security.md) and [privacy](privacy.md) for that boundary.

To share a device, sign out before another person signs in. Tabs in the same browser profile share the same login session. Use separate browser profiles when two people need to stay signed in on the same device.

## Revoke an unused invitation

In Workspace, find the pending invitation and choose **Revoke invitation**. The link can no longer create an account. Revocation does not remove someone who has already joined or disconnect their mailboxes.

Local password reset, user removal, and ownership transfer do not currently have a management UI. Keep your login in a password manager and preserve operator access and backups. Do not delete data or regenerate encryption keys to recover a login.

## Use Dakia on another device

Open the same HTTPS address and sign in with your existing Dakia username and password. You do not need another invitation or another mailbox setup. The host must remain running for syncing and scheduled sending.

You can install the web app through your browser's installation controls. Installation does not provide an offline mailbox. Notification setup requires the host's optional push configuration and browser permission; see [operations](../deploy/README.md) and [worker configuration](../workers/README.md).

## Troubleshooting

| Problem | What to check |
| --- | --- |
| The page cannot be reached | The service and HTTPS proxy are running; DNS is correct; you are on the required LAN or VPN. An invitation does not change network permissions. |
| Sign-in fails or returns to the login screen | Use the configured HTTPS address. Check the username and password, and ensure the proxy preserves the Origin header. |
| Too many attempts | Wait a minute before retrying. Repeated failures for one username are limited separately from other users, with an additional instance-wide cap. |
| The invitation is invalid or used | Ask the owner for a fresh link. Used links cannot create a second account. |
| No invitation controls appear | They are available only to the local owner. In OIDC mode, users are managed through the identity provider. |
| A mailbox needs attention | Use Reconnect in Workspace. Check provider credentials/app passwords, IMAP access, server settings, and outbound IMAP/SMTP connectivity. |

If sign-in uses an external identity provider, follow that provider's login and account-access process. Local invitations are disabled in OIDC mode, and local and OIDC identities are not automatically linked.

Snooze removes an email from Inbox until the time you choose. Snoozed shows the email count and each email's return time. Choose **Move back to Inbox** to bring an email back sooner. Both actions show a confirmation after they succeed.
