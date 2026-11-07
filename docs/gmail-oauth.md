# Gmail OAuth for a personal installation

Each operator supplies a Google OAuth client for their own Dakia installation. Dakia does not ship shared Google credentials. Google authorization connects a mailbox to an already signed-in Dakia user; it does not replace local or OIDC login.

## Set up Google

1. Create a Google Cloud project for your personal installation. Use a separate development project and disposable Dakia data when testing changes. Do not reuse production client credentials or data.
2. Configure Google Auth Platform branding and audience. For a personal Gmail account, choose External, leave the development app in Testing, and add your Google account under Audience → Test users.
3. Add `openid`, `email`, and `https://mail.google.com/` under Data Access. Dakia's existing IMAP/SMTP mail transport needs the full mail scope. Calendar and contacts scopes are not requested.
4. Create an OAuth client of type **Web application**. Register exactly `<DAKIA_ORIGIN>/api/accounts/gmail/callback`, for example `https://mail.example.com/api/accounts/gmail/callback`. No JavaScript origin is needed for this server flow.
5. Keep the client ID and secret private. Configure Dakia as below and restart it.

Google's [IMAP/SMTP OAuth documentation](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol) describes the required scope. The [web server authorization guide](https://developers.google.com/identity/protocols/oauth2/web-server) covers redirect registration and offline access. Gmail API access and Pub/Sub are not used by this implementation; existing IMAP synchronization and IDLE continue working.

Google normally expires refresh tokens after seven days for an External app in Testing when mail scopes are requested. Plan to reconnect during development. Publishing status and verification are separate settings. Before changing either for a personal installation, read Google's [OAuth verification exceptions](https://support.google.com/cloud/answer/13464321) and [refresh-token expiration rules](https://developers.google.com/identity/protocols/oauth2#expiration). This setup does not make a shared public Dakia OAuth service verified.

## Configure Dakia

For the root Compose package, set the optional values in `.env`:

```dotenv
DAKIA_GMAIL_CLIENT_ID=your-client.apps.googleusercontent.com
DAKIA_GMAIL_CLIENT_SECRET=your-client-secret
```

Restrict `.env` permissions and keep it outside version control. For a secret file instead, keep only the client ID in `.env`, add `DAKIA_GMAIL_SECRET_HOST_FILE` with its absolute host path, and create `compose.override.yaml`:

```yaml
services:
  dakia:
    environment:
      DAKIA_GMAIL_SECRET_FILE: /run/secrets/dakia-gmail
    volumes:
      - ${DAKIA_GMAIL_SECRET_HOST_FILE:?Set a private absolute file path}:/run/secrets/dakia-gmail:ro
```

The mounted file must contain only the client secret, readable by container UID 1000. `DAKIA_GMAIL_SECRET_FILE` takes precedence over the environment secret. A standalone server accepts the same settings. Neither value is needed when using an app password. Configuring only an ID or only a secret fails startup.

Use a build containing this feature. An older pinned image does not gain OAuth support from configuration alone.

## Connect or reconnect

In Dakia, choose **Connect account**, select **Gmail**, and enter an account name and email. Choose **Continue with Google**, sign in to that exact Google account, and allow mail access. Dakia checks the verified email returned by Google before saving anything. Reconnecting an existing Gmail account preserves its local messages, drafts, and identity. Selecting a different Google account is rejected.

Dakia saves the refresh token in its existing encrypted account credential store. The operator's client secret remains in server configuration. Access tokens are cached only in memory and renewed before expiry for new IMAP/SMTP connections. Token-service outages can be retried; a revoked grant requires reconnecting. Changing the configured client ID requires reconnecting accounts authorized against the old client. Rotating the secret for the same client ID requires restarting Dakia.

A cancelled or expired authorization returns to Dakia with a readable error. A failed mail probe can leave a saved account in Workspace for retry or reconnect. Removing an account deletes its local credentials; it does not revoke Google's grant or delete mail. You can revoke the grant separately in your Google account's third-party connections.

Gmail's label visibility and folder-size limits still affect IMAP coverage. Check the existing mailbox settings after connecting. An OAuth token does not bypass those settings.

## Development checks

Use a dedicated Google Cloud project, a separate OAuth web client, a separate data directory and credential key, and a local HTTPS address. Register that development address's exact callback in the development client. Keep downloaded client JSON and secrets outside the repository.

Run the OAuth server and provider fixture tests, then the Chrome keyboard tests:

```sh
cargo test -p dakia-server gmail_oauth
cargo test -p dakia-mail-provider --test oauth_fixture
npm run test:browser -- gmail-oauth.spec.ts
```

The fixtures cover session binding, CSRF, replay and expiry, wrong-account rejection, encrypted credential storage, access-token caching/renewal, revoked grants, and XOAUTH2 wire authentication. Browser fixtures check the dialog and app-password fallback across themes and viewports. They do not replace a real Google consent and mailbox connection check using the development project. For that check, authorize a test mailbox, reload Dakia, restart the isolated server, and confirm that it can renew access and probe IMAP/SMTP without sending a message.
