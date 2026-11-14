# Privacy

Dakia runs on the server you choose and connects to your existing email providers. Self-hosting gives the operator control over storage and processing; it doesn't hide mail from the provider or from someone with administrative access to the host.

## Where your mail goes

| Activity | Where it happens |
| --- | --- |
| Sync, search, drafts, and sending | Your Dakia server and the connected email provider |
| Personal/Automated classification and detected details | Local rules on your Dakia server |
| On-device translation | Your browser, using downloaded language packs |
| Server translation, including enabled fallback | The translation worker on your Dakia server |
| Recent-mail cache | This browser's local storage, scoped to the signed-in user and accounts |

Translation doesn't use an external translation API. Public language packs are fetched by your server from pinned Mozilla sources and delivered to your browser when needed. Pack downloads contain public model files, not your email text. The installation owner chooses the translation location; server translation is the default. See [translation settings](on-device-translation.md).

## Browser storage and images

Recent folders and message bodies can remain saved in this browser. Signing out or removing an account clears the mail cache and tells other updated tabs to reload. Use separate browser profiles for people sharing a device. Public language packs remain after sign-out and can be removed in Workspace. Clearing site data removes saved mail, packs, and preferences. See [cache limits](persistent-local-cache.md).

Received HTML is sanitized and isolated. Remote images follow your per-browser privacy preferences and use the server's image proxy. Loading an image can still contact its sender's image host from your server, including tracking URLs. Following a link opens its destination and is subject to that site's privacy practices.

## Connections beyond your mail provider

Google and Microsoft sign-in contact those providers when you connect or reconnect a mailbox. OIDC installations also contact their identity provider for Dakia login.

The owner's update check contacts GitHub for published releases without including mailbox or user data. Operators can disable it with `DAKIA_RELEASE_CHECKS=false`.

Optional Web Push uses the browser's supported push service. Push payloads omit sender, subject, and body. The signed-in browser may fetch a message summary and show it in a notification, so consider lock-screen visibility before enabling notifications.

## Protecting stored mail

Mailbox credentials are encrypted with a separate installation key. Message content isn't fully encrypted at rest by Dakia. Protect the host, browser profile, persistent volumes, and backups; use disk encryption where appropriate. Separate app logins don't prevent the host administrator from reading files or backups.

Don't publish logs, diagnostic files, backups, or screenshots containing private mail. For vulnerability reports and protection details, see [security](security.md).
