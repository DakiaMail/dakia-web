# Gmail sign-in

Connect Gmail with **Continue with Google**, or use an app password when your Google account permits it. Google sign-in needs an OAuth client registered by your installation's operator. It authorizes a mailbox; your Dakia login stays separate.

## Register a Google client

1. Create a Google Cloud project for your installation.
2. Configure Google Auth Platform branding and audience. For a personal Gmail account, choose External. While testing, add your account under Audience → Test users.
3. Add `openid`, `email`, and `https://mail.google.com/` under Data Access. Dakia uses Gmail's IMAP/SMTP transport and needs the full mail scope. It doesn't request calendar or contacts access.
4. Create an OAuth client of type **Web application**. Register exactly `<DAKIA_ORIGIN>/api/accounts/gmail/callback`, for example `https://mail.example.com/api/accounts/gmail/callback`. No JavaScript origin is needed for this server flow.
5. Save the client ID and secret privately.

Google limits refresh tokens for External apps in Testing to seven days when Gmail scopes are requested. Expect to reconnect while testing. Review [Google's token expiration rules](https://developers.google.com/identity/protocols/oauth2#expiration) and [restricted-scope requirements](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification) before sharing an OAuth application more widely.

## Configure Dakia

For the root Compose package, set these values in its private `.env`:

```dotenv
DAKIA_GMAIL_CLIENT_ID=your-client.apps.googleusercontent.com
DAKIA_GMAIL_CLIENT_SECRET=your-client-secret
```

Recreate the container with your normal Compose options. Both settings are required together; partial configuration fails startup.

To use a secret file instead, omit the environment secret and merge these settings into the existing private `compose.override.yaml`. Put `dakia` under `services`, alongside any `caddy` service, and retain the top-level volumes. If no override exists, create it:

```yaml
services:
  dakia:
    environment:
      DAKIA_GMAIL_SECRET_FILE: /run/secrets/dakia-gmail
    volumes:
      - /absolute/private/gmail-client-secret:/run/secrets/dakia-gmail:ro
```

The file must contain only the client secret and be readable by container UID 1000. The file takes precedence over the environment secret. Keep the client ID in `.env`. Include the override explicitly if your installation already uses explicit `-f` Compose options.

## Connect or reconnect

In Dakia, choose **Connect account**, enter the Gmail address, choose Gmail, and select **Continue with Google**. Authorize the same Google account and allow mail access. Dakia stores the refresh token encrypted on your server and renews access automatically while the grant remains valid.

If a grant expires or is revoked, use **Reconnect** in Workspace. Check the registered callback, client credentials, test-user list, requested permissions, and Google's error message if sign-in fails. Provider or organization policies can restrict access.

Finishing Google's screens does not guarantee that reconnection succeeded. Dakia must receive the callback, confirm the Google account, save the new authorization, and check the Gmail connection. A failed callback shows an error on return to Dakia. Operator logs record only a safe failure category, such as `google_token_exchange_failed`, `google_profile_failed`, or `google_offline_access_required`. A later Gmail connection failure is reported separately.

Token-renewal logs distinguish `invalid_grant` (expired or revoked authorization) from client configuration failures. Google does not say which expiration or revocation cause applies. A fresh sign-in is required for `invalid_grant`; retrying the saved authorization cannot repair it.

Choose **Use an app password** to connect without Google OAuth when that option is available for your account. A normal Google password may not work for IMAP/SMTP.

## Development

Use a separate Google project, loopback test instance, and disposable data directory. Never copy real mailbox credentials into fixtures or publish authorization URLs containing private state. The packaged OAuth smoke check tests the flow with synthetic credentials; it doesn't establish acceptance with a live Gmail account.
