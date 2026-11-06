use crate::*;
use lettre::{
    message::{
        header::{ContentTransferEncoding, ContentType},
        Attachment, Body, Mailbox, MultiPart, SinglePart,
    },
    Message,
};

pub struct Compose {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    /// Stable ID generated once when the immutable submission snapshot is created.
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub attachments: Vec<ComposeAttachment>,
}
pub struct ComposeAttachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}
/// Store these bytes with the immutable submission, not rebuild on each retry.
/// Fields are private to prevent constructing an envelope with an unverified From.
pub struct PreparedMessage {
    pub(crate) envelope: lettre::address::Envelope,
    pub(crate) raw: Vec<u8>,
    pub(crate) account_id: String,
    pub(crate) primary_address: String,
}
impl PreparedMessage {
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
    pub fn recipients(&self) -> Vec<String> {
        self.envelope.to().iter().map(ToString::to_string).collect()
    }
    /// Restore the exact persisted bytes and envelope after restart. `recipients`
    /// must come from the immutable server snapshot, including Bcc, never HTTP.
    pub fn restore(
        config: &Config,
        raw: Vec<u8>,
        recipients: Vec<String>,
        max_encoded_bytes: usize,
    ) -> Result<Self> {
        if raw.len() > max_encoded_bytes {
            return Err(error(
                ErrorKind::InvalidInput,
                "persisted MIME exceeds provider limit",
            ));
        }
        let parsed = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| error(ErrorKind::InvalidInput, "invalid persisted MIME"))?;
        let primary = parse_mailbox(&config.primary_address)?;
        let from = parsed
            .from()
            .and_then(|a| a.first())
            .and_then(|a| a.address());
        if from != Some(primary.email.as_ref())
            || parsed.from().is_none_or(|a| a.iter().count() != 1)
            || parsed.bcc().is_some()
            || parsed.message_id().is_none()
        {
            return Err(error(
                ErrorKind::InvalidInput,
                "persisted MIME identity or headers invalid",
            ));
        }
        let to = recipients
            .iter()
            .map(|r| parse_mailbox(r).map(|m| m.email))
            .collect::<Result<Vec<_>>>()?;
        let envelope = lettre::address::Envelope::new(Some(primary.email), to)
            .map_err(|_| error(ErrorKind::InvalidInput, "invalid persisted envelope"))?;
        Ok(Self {
            envelope,
            raw,
            account_id: config.account_id.clone(),
            primary_address: config.primary_address.clone(),
        })
    }
}
pub fn assemble(
    config: &Config,
    draft: Compose,
    max_encoded_bytes: usize,
) -> Result<PreparedMessage> {
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Err(error(
            ErrorKind::InvalidInput,
            "at least one recipient required",
        ));
    }
    if !draft.message_id.starts_with('<')
        || !draft.message_id.ends_with('>')
        || !draft.message_id.contains('@')
        || draft.message_id.chars().any(char::is_whitespace)
    {
        return Err(error(
            ErrorKind::InvalidInput,
            "stable RFC Message-ID required",
        ));
    }
    for value in [&draft.subject, &draft.message_id]
        .into_iter()
        .chain(draft.in_reply_to.as_ref())
        .chain(draft.references.as_ref())
    {
        if value.contains(['\r', '\n']) {
            return Err(error(ErrorKind::InvalidInput, "header injection rejected"));
        }
    }
    let total = draft
        .attachments
        .iter()
        .try_fold(0usize, |n, a| n.checked_add(a.bytes.len()))
        .ok_or_else(|| error(ErrorKind::InvalidInput, "attachment size overflow"))?;
    if total > 20 * 1024 * 1024 {
        return Err(error(ErrorKind::InvalidInput, "attachments exceed 20 MiB"));
    }
    let from: Mailbox = config
        .primary_address
        .parse()
        .map_err(|_| error(ErrorKind::InvalidInput, "invalid primary sending identity"))?;
    let mut builder = Message::builder()
        .from(from)
        .message_id(Some(draft.message_id))
        .subject(draft.subject);
    for address in draft.to {
        builder = builder.to(parse_mailbox(&address)?);
    }
    for address in draft.cc {
        builder = builder.cc(parse_mailbox(&address)?);
    }
    for address in draft.bcc {
        builder = builder.bcc(parse_mailbox(&address)?);
    }
    if let Some(v) = draft.in_reply_to {
        builder = builder.in_reply_to(message_id_header(&v, true)?);
    }
    if let Some(v) = draft.references {
        builder = builder.references(message_id_header(&v, false)?);
    }
    let mut body = MultiPart::mixed().multipart(if let Some(html) = draft.html {
        MultiPart::alternative()
            .singlepart(SinglePart::plain(draft.text))
            .singlepart(SinglePart::html(html))
    } else {
        MultiPart::alternative().singlepart(SinglePart::plain(draft.text))
    });
    for a in draft.attachments {
        let content_type: ContentType = a
            .content_type
            .parse()
            .map_err(|_| error(ErrorKind::InvalidInput, "invalid attachment content type"))?;
        if a.filename.chars().any(char::is_control) {
            return Err(error(
                ErrorKind::InvalidInput,
                "invalid attachment filename",
            ));
        }
        // 7bit transport canonicalizes LF to CRLF. Attachments are opaque files,
        // including text files, and must survive SMTP byte-for-byte.
        let encoded = Body::new_with_encoding(a.bytes, ContentTransferEncoding::Base64)
            .map_err(|_| error(ErrorKind::InvalidInput, "attachment encoding failed"))?;
        body = body.singlepart(Attachment::new(a.filename).body(encoded, content_type));
    }
    let message = builder
        .multipart(body)
        .map_err(|_| error(ErrorKind::InvalidInput, "MIME assembly failed"))?;
    let raw = message.formatted();
    if raw.len() > max_encoded_bytes {
        return Err(error(
            ErrorKind::InvalidInput,
            "encoded message exceeds provider limit",
        ));
    }
    Ok(PreparedMessage {
        envelope: message.envelope().clone(),
        raw,
        account_id: config.account_id.clone(),
        primary_address: config.primary_address.clone(),
    })
}
// mail-parser returns inner IDs; lettre writes these headers verbatim.
fn message_id_header(value: &str, single: bool) -> Result<String> {
    let mut ids = Vec::new();
    for token in value.split_ascii_whitespace() {
        let id = token
            .strip_prefix('<')
            .and_then(|s| s.strip_suffix('>'))
            .unwrap_or(token);
        if id.contains(['<', '>'])
            || id.chars().any(char::is_control)
            || !id.split_once('@').is_some_and(|(local, domain)| {
                !local.is_empty() && !domain.is_empty() && !domain.contains('@')
            })
        {
            return Err(error(ErrorKind::InvalidInput, "invalid reply Message-ID"));
        }
        ids.push(format!("<{id}>"));
    }
    if ids.is_empty() || (single && ids.len() != 1) {
        return Err(error(ErrorKind::InvalidInput, "invalid reply Message-ID"));
    }
    Ok(ids.join(" "))
}
fn parse_mailbox(s: &str) -> Result<Mailbox> {
    s.parse()
        .map_err(|_| error(ErrorKind::InvalidInput, "invalid recipient address"))
}
