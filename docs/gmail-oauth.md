# Gmail OAuth for a personal installation

Gmail OAuth connection requires creating your own Google Cloud project (don't worry, it is easy). You can also avoid this setup and just use an app-password if available. 

## Set up Google

1. Create a Google Cloud project for your personal installation. 
2. Configure Google Auth Platform branding and audience. For a personal Gmail account, choose External, leave the app in Testing, and add your Google account under Audience → Test users.
3. Add `openid`, `email`, and `https://mail.google.com/` under Data Access. Dakia's existing IMAP/SMTP mail transport needs the full mail scope. Calendar and contacts scopes are not requested.
4. Create an OAuth client of type **Web application**. Register exactly `<DAKIA_ORIGIN>/api/accounts/gmail/callback`, for example `https://mail.example.com/api/accounts/gmail/callback`. No JavaScript origin is needed for this server flow.
5. Keep the client ID and secret private. Configure Dakia as below and restart it.

## Configure Dakia

For the root Compose package, set the optional values in `.env`:

```dotenv
DAKIA_GMAIL_CLIENT_ID=your-client.apps.googleusercontent.com
DAKIA_GMAIL_CLIENT_SECRET=your-client-secret
```

The mounted file must contain only the client secret, readable by container UID 1000. `DAKIA_GMAIL_SECRET_FILE` takes precedence over the environment secret. 
