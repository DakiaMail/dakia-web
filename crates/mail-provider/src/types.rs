use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Security {
    Tls,
    StartTls,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub security: Security,
}
/// Credentials deliberately have no Debug or Serialize implementation.
pub struct Credentials {
    pub username: String,
    pub password: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderKind {
    Gmail,
    Imap,
}
#[derive(Clone)]
pub struct Config {
    pub account_id: String,
    pub primary_address: String,
    pub kind: ProviderKind,
    pub imap: Endpoint,
    pub smtp: Endpoint,
    pub timeout: Duration,
    /// Must be explicitly confirmed by setup after checking Gmail's IMAP exposure settings.
    pub full_imap_exposure_confirmed: bool,
    pub folders: FolderMapping,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct FolderMapping {
    pub sent: Option<String>,
    pub archive: Option<String>,
    pub trash: Option<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorKind {
    Retryable,
    Authentication,
    Unsupported,
    InvalidInput,
    StaleReference,
    Uncertain,
}
#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
#[error("{kind:?}: {message}")]
pub struct ProviderError {
    pub kind: ErrorKind,
    pub message: String,
}
pub type Result<T> = std::result::Result<T, ProviderError>;
pub fn error(kind: ErrorKind, message: &str) -> ProviderError {
    ProviderError {
        kind,
        message: message.into(),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_id: Option<String>,
    pub account_id: String,
    pub folder: String,
    pub uidvalidity: u32,
    pub uid: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Identity {
    Gmail {
        account_id: String,
        message_id: u64,
    },
    Imap(MessageRef),
    Graph {
        account_id: String,
        message_id: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub reference: MessageRef,
    pub identity: Identity,
    pub gmail_thread_id: Option<u64>,
    pub labels: Vec<String>,
    pub seen: bool,
    pub starred: bool,
    pub size: u32,
    /// Provider arrival time, not the sender-controlled RFC Date header.
    pub internal_date: String,
    pub headers: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub selectable: bool,
    pub attributes: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Probe {
    pub capabilities: Vec<String>,
    pub folders: Vec<Folder>,
    pub full_search_coverage: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Search {
    /// Header-only conversation discovery, never a body or subject search.
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub gmail_thread: Option<u64>,
    pub text: Option<String>,
    /// Each unit is a required literal word or quoted phrase. Empty retains
    /// the legacy single TEXT-string behavior for existing provider callers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_units: Vec<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub since: Option<String>,
    pub before: Option<String>,
    pub seen: Option<bool>,
    pub starred: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageToken {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_next: Option<String>,
    pub account_id: String,
    pub folder: String,
    pub uidvalidity: u32,
    pub upper_uid: u32,
    pub before_uid: u32,
    pub query: Search,
    /// Captured before the first page, never a later page's high watermark.
    #[serde(default)]
    pub modseq: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub rows: Vec<Metadata>,
    pub next: Option<PageToken>,
    pub uidvalidity: u32,
    pub full_coverage: bool,
}
/// Incremental arrivals, scoped to one account/mailbox/UID epoch. This is not a
/// full membership snapshot and can never authorize pruning.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeadCheckpoint {
    pub account_id: String,
    pub folder: String,
    pub uidvalidity: u32,
    pub uid: u32,
}
#[derive(Clone, Debug)]
pub struct HeadPage {
    pub rows: Vec<Metadata>,
    pub checkpoint: HeadCheckpoint,
}

/// Mutable observations never stand in for missing immutable headers.
#[derive(Clone, Debug)]
pub struct MutableMetadata {
    pub reference: MessageRef,
    pub identity: Identity,
    pub labels: Vec<String>,
    pub seen: bool,
    pub starred: bool,
}
#[derive(Clone, Debug)]
pub enum MetadataChange {
    New(Metadata),
    Mutable(MutableMetadata),
    Unchanged(MessageRef),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModseqCheckpoint {
    pub account_id: String,
    pub folder: String,
    pub uidvalidity: u32,
    pub modseq: u64,
}
pub struct ReconcilePage {
    pub rows: Vec<MetadataChange>,
    pub next: Option<PageToken>,
    pub uidvalidity: u32,
    pub full_coverage: bool,
    pub checkpoint: Option<ModseqCheckpoint>,
}
#[derive(Clone, Debug)]
pub enum Mutation {
    Seen(bool),
    Starred(bool),
    Archive,
    Trash,
    Spam,
    Move(String),
    Copy(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MutationResult {
    pub destination: Option<String>,
    pub destination_reference: Option<MessageRef>,
    pub requires_rescan: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubmissionOutcome {
    Accepted {
        response: String,
    },
    Failed {
        kind: ErrorKind,
    },
    /// A definitive refusal or pre-request failure; safe to defer the same immutable MIME.
    Deferred {
        retry_after_secs: u64,
    },
    DeliveryUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SentOutcome {
    ProviderManaged,
    AlreadyPresent(String),
    Appended(String),
    FilingUnknown,
}

pub fn quote(value: &str) -> Result<String> {
    if value.chars().any(|c| c.is_control()) {
        return Err(error(
            ErrorKind::InvalidInput,
            "control character in IMAP argument",
        ));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}
impl Search {
    pub fn imap_query(&self) -> Result<String> {
        let mut terms = vec!["ALL".to_owned()];
        if let Some(id) = &self.conversation_id {
            let id = quote(id)?;
            terms.push(format!(
                "OR HEADER Message-ID {id} OR HEADER References {id} HEADER In-Reply-To {id}"
            ));
        }
        if let Some(id) = self.gmail_thread {
            terms.push(format!("X-GM-THRID {id}"));
        }
        if self.text_units.len() > 2048
            || self
                .text_units
                .iter()
                .fold(0usize, |n, s| n.saturating_add(s.len() + 1))
                > 4097
        {
            return Err(error(
                ErrorKind::InvalidInput,
                "search text exceeds query budget",
            ));
        }
        if self.text_units.is_empty() {
            if let Some(text) = &self.text {
                terms.push(format!("TEXT {}", quote(text)?));
            }
        } else {
            for text in &self.text_units {
                if text.is_empty() {
                    return Err(error(ErrorKind::InvalidInput, "empty search text unit"));
                }
                terms.push(format!("TEXT {}", quote(text)?));
            }
        }
        for (key, value) in [
            ("FROM", &self.from),
            ("TO", &self.to),
            ("SUBJECT", &self.subject),
        ] {
            if let Some(value) = value {
                let value = quote(value)?;
                terms.push(if key == "TO" {
                    // Match the local recipient field, including available Cc/Bcc.
                    format!("(OR TO {value} OR CC {value} BCC {value})")
                } else {
                    format!("{key} {value}")
                });
            }
        }
        for (key, value) in [("SINCE", &self.since), ("BEFORE", &self.before)] {
            if let Some(value) = value {
                if !valid_date(value) {
                    return Err(error(ErrorKind::InvalidInput, "date must be DD-Mon-YYYY"));
                }
                terms.push(format!("{key} {value}"));
            }
        }
        if let Some(v) = self.seen {
            terms.push(if v { "SEEN" } else { "UNSEEN" }.into());
        }
        if let Some(v) = self.starred {
            terms.push(if v { "FLAGGED" } else { "UNFLAGGED" }.into());
        }
        Ok(terms.join(" "))
    }
}
fn valid_date(s: &str) -> bool {
    let p: Vec<_> = s.split('-').collect();
    p.len() == 3
        && p[0].parse::<u8>().is_ok_and(|v| (1..=31).contains(&v))
        && [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ]
        .contains(&p[1])
        && p[2].len() == 4
        && p[2].parse::<u16>().is_ok()
}

/// Supplies a current bearer token for each new authenticated connection.
#[async_trait::async_trait]
pub trait AccessToken: Send + Sync {
    async fn access_token(&self) -> Result<String>;
}

#[cfg(test)]
mod search_unit_tests {
    use super::*;
    #[test]
    fn provider_units_preserve_phrases_and_match_separate_words_independently() {
        let query = Search {
            text: Some("quarterly report budget".into()),
            text_units: vec!["quarterly report".into(), "budget".into()],
            from: Some("maya@example.invalid".into()),
            ..Default::default()
        };
        assert_eq!(
            query.imap_query().unwrap(),
            "ALL TEXT \"quarterly report\" TEXT \"budget\" FROM \"maya@example.invalid\""
        );
        let old = Search {
            text: Some("quarterly report budget".into()),
            ..Default::default()
        };
        assert_eq!(
            old.imap_query().unwrap(),
            "ALL TEXT \"quarterly report budget\""
        );
        let restored: Search =
            serde_json::from_str(&serde_json::to_string(&query).unwrap()).unwrap();
        assert_eq!(restored, query);
    }
    #[test]
    fn recipient_scope_matches_available_to_cc_and_bcc_without_weakening_other_filters() {
        let query = Search {
            to: Some("Finance Team".into()),
            subject: Some("Budget".into()),
            seen: Some(false),
            ..Default::default()
        };
        assert_eq!(query.imap_query().unwrap(),"ALL (OR TO \"Finance Team\" OR CC \"Finance Team\" BCC \"Finance Team\") SUBJECT \"Budget\" UNSEEN");
        let injected = Search {
            to: Some("x\r\nEXPUNGE".into()),
            ..Default::default()
        };
        assert!(injected.imap_query().is_err());
    }
    #[test]
    fn provider_units_are_bounded_and_never_interpolate_commands() {
        let bad = Search {
            text_units: vec!["ok\r\nEXPUNGE".into()],
            ..Default::default()
        };
        assert!(bad.imap_query().is_err());
        let big = Search {
            text_units: vec!["ab".into(); 2048],
            ..Default::default()
        };
        assert!(big.imap_query().is_err());
        let empty = Search {
            text_units: vec![String::new()],
            ..Default::default()
        };
        assert!(empty.imap_query().is_err());
        let escaped = Search {
            text_units: vec!["quoted \"text\"".into()],
            ..Default::default()
        };
        assert_eq!(
            escaped.imap_query().unwrap(),
            "ALL TEXT \"quoted \\\"text\\\"\""
        );
    }
}
