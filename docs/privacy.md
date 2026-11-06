# Privacy

Dakia runs on infrastructure you control. This initial overview describes the current design; a detailed network and retention inventory is planned.

Mail sync and sending connect to the email providers you configure. Classification and translation run on your server. Public model files may be downloaded from their upstream hosts. An optional OIDC provider handles sign-in when configured. Optional Web Push uses browser push infrastructure with generic notification payloads rather than subjects or message bodies.

Dakia stores mail caches, drafts, attachments, settings, and operational records locally. Credential encryption does not encrypt all stored mail. Your host administrator and anyone with sufficient host or backup access can read that data.

Remote images can contact sender-controlled services through Dakia's proxy when allowed. Proxying hides the browser's direct connection but does not eliminate tracking: unique image URLs can still identify a message or recipient.

The service worker caches public application assets, not API responses, messages, or drafts. Operational diagnostics are retained locally and may contain sensitive metadata; keep diagnostics and backups private. A field-by-field diagnostics review and retention guide are planned.

A remote image policy, provider-side retention, and copies in backups can affect deletion. Removing something from Dakia does not necessarily erase every copy held by your provider or backups.
