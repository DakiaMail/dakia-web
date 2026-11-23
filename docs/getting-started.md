# Getting started

Setting up your own installation? Start with [hosting](hosting.md), then [create the owner login](#create-the-first-login).

Joining someone else's installation? Open their invitation link and follow [Join an installation](#join-an-installation). You don't need Docker or access to their server.

## Create the first login

The first person to set up a new installation becomes its owner. Open Dakia and choose **Sign in**, enter a username and a password with at least 12 characters, then choose **Continue**.

The current image needs no setup code. Keep the network restrictions from the hosting guide in place until you have created the login. The first person who reaches an unclaimed installation can become its owner.

Your Dakia login is separate from your email provider's login. Once owner setup is complete, the sign-in page asks only for your username and password.

## Connect an email account

1. Choose **Connect account** in the welcome screen or Workspace.
2. Give the mailbox a name, such as Personal or Work, and enter its email address.
3. Choose the connection method for your provider:

| Mailbox | How to connect |
| --- | --- |
| Gmail | Choose **Continue with Google** if the operator has [configured Google sign-in](gmail-oauth.md). Sign in to the same Gmail address and allow mail access. Choose **Use an app password** instead if your account supports one. |
| Outlook.com, Hotmail, or Microsoft 365 | Choose **Microsoft (Outlook / Microsoft 365)**, then **Continue with Microsoft**. The operator must first [configure Microsoft sign-in](microsoft-oauth.md). |
| Another IMAP/SMTP provider | Use Automatic discovery. If it cannot find the settings, enter the provider's incoming and outgoing hostnames, ports, username, and encryption settings manually. Use its app password when required. |

For password-based connections, enter the mailbox or app password and choose **Connect account**. This is separate from your Dakia password. Dakia saves the connection and checks it; if the check fails, correct the settings shown in the form and retry.

For IMAP/SMTP, match your provider's TLS or required STARTTLS settings. The host needs outbound access to both mail servers. Microsoft connections use outbound HTTPS to Microsoft sign-in and Graph instead. You don't need MX changes or an incoming mail server.

Repeat this for additional mailboxes. **Unified inbox** shows your connected accounts together, and the account list lets you open one mailbox at a time. Workspace provides mailbox settings, reconnection, and removal controls.

## Check your first mailbox

Mail can appear before the full history has finished syncing. Keep the server running; older messages and search results may still be incomplete during import. The syncing indicator shows when an account is working. Repeated connection errors need attention in Workspace.

Open a message and check its attachments. Send a short test message to an address you control and confirm it arrives. If sending reports an uncertain result, check the recipient and provider's Sent folder before trying again.

## Find and organize mail

Your inbox opens on **Personal** and remembers your last tab. **Automated** groups generated mail and offers filters such as Transactions, Orders, Shipping, and Newsletters. **All** shows both. **Needs attention** surfaces important items across those tabs. Use **More actions** in a message to correct a classification or remember a sender's preference for that mailbox.

Choose **Translate to English** while reading a supported message. The installation owner chooses whether translation runs on devices, on the server, or on devices with server fallback in **Workspace → Translation**. The first on-device use downloads a language pack. See [translation settings](on-device-translation.md).

Snooze removes a message from Inbox until your chosen time. **Snoozed** shows return times and lets you choose **Move back to Inbox** to bring one back sooner. Use **Send later** in the composer to schedule a message. Keep the host running for these background actions.

For more, see [features](features.md). Press **?** outside an editing field to open the keyboard shortcut reference. Workspace lets you switch shortcuts and hints on or off.

## Invite another person

Inviting others is optional. Only the local owner sees invitation controls.

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

You can install the web app through your browser's installation controls. Installation does not download a complete offline mailbox. Recent mail can be available within an already-authorized session; a fresh startup needs online session checks. See [saved mail](persistent-local-cache.md). Notification setup requires the host's optional push configuration and browser permission; see [operations](../deploy/README.md) and [worker configuration](../workers/README.md).

## Connect an AI assistant

Open **Workspace → Agent connections**, choose an assistant, and copy the server URL. Approve the connection in your browser. Sending and deletion start off, and you can change each permission or revoke the connection in Workspace. Follow the [client setup guide](mcp-connections.md) for ChatGPT, Claude, Hermes Agent, and other MCP clients.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| The page cannot be reached | The service and HTTPS proxy are running; DNS is correct; you are on the required LAN or VPN. An invitation does not change network permissions. |
| Sign-in fails or returns to the login screen | Use the configured HTTPS address. Check the username and password, and ensure the proxy preserves the Origin header. |
| Too many attempts | Wait a minute before retrying. Repeated failures for one username are limited separately from other users, with an additional instance-wide cap. |
| The invitation is invalid or used | Ask the owner for a fresh link. Used links cannot create a second account. |
| No invitation controls appear | They are available only to the local owner. In OIDC mode, users are managed through the identity provider. |
| A mailbox needs attention | Use Reconnect in Workspace. Check provider credentials/app passwords, IMAP access, server settings, and outbound IMAP/SMTP connectivity. |

If the setup page asks for a setup code, you're running an older image. The operator can read it privately with `sudo docker compose exec -T dakia cat /secrets/setup-token` from the installation directory. Don't post the code in an issue. Follow [upgrading](upgrading.md) when ready to move to a newer image.

If sign-in uses an external identity provider, follow that provider's login and account-access process. Local invitations are disabled in OIDC mode, and local and OIDC identities are not automatically linked.
