//! Shared wire schema. Generate the client with `cargo test -p dakia-domain`.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

macro_rules! dto {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
        #[ts(export, export_to = "../../../packages/api-types/src/generated/")]
        pub struct $name { $(pub $field: $ty),* }
    };
}
macro_rules! values {
    ($name:ident { $($v:ident),* }) => {
        #[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        #[ts(export, export_to = "../../../packages/api-types/src/generated/")]
        pub enum $name { $($v),* }
    };
}
values!(ProviderKind { Gmail, Imap, Microsoft });
values!(AccountState {
    Connecting,
    Ready,
    Reconnect,
    Error
});
values!(CategoryStatus {
    Pending,
    Ready,
    Failed
});
values!(SubmissionState {
    Scheduled,
    Claimed,
    Submitting,
    Accepted,
    Failed,
    DeliveryUnknown,
    Cancelled
});
values!(FilingStatus {
    Pending,
    Filed,
    NotRequired,
    Failed
});
values!(OperationStatus {
    Pending,
    Applied,
    Failed
});
values!(CoverageState {
    Complete,
    Partial,
    Pending,
    Error
});
values!(JobKind {
    Classify,
    Translate
});
values!(JobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled
});

dto!(Account {
    id: String,
    email: String,
    display_name: String,
    provider: ProviderKind,
    state: AccountState,
    revision: u32
});
dto!(MailboxSettings {
    full_imap_exposure_confirmed: bool,
    sent_folder: Option<String>,
    archive_folder: Option<String>,
    trash_folder: Option<String>,
    received_cache_budget: u32
});
impl Default for MailboxSettings {
    fn default() -> Self {
        Self {
            full_imap_exposure_confirmed: false,
            sent_folder: None,
            archive_folder: None,
            trash_folder: None,
            received_cache_budget: 1024 * 1024 * 1024,
        }
    }
}
dto!(Address { email: String, name: Option<String> });
dto!(RemoteImageRequest { url: String });
dto!(Message { id: String, account_id: String, thread_id: String, provider_id: String, folder_ids: Vec<String>, sender: Address, to: Vec<Address>, subject: String, preview: String, received_at: String, unread: bool, starred: bool, category: Option<String>, category_status: CategoryStatus, revision: u32 });
dto!(Attachment {
    id: String,
    filename: String,
    content_type: String,
    size_bytes: u32
});
dto!(Draft { id: String, account_id: String, revision: u32, to: Vec<Address>, cc: Vec<Address>, bcc: Vec<Address>, subject: String, text: String, html: String, in_reply_to: Option<String>, references: Vec<String>, attachments: Vec<Attachment>, saved_at: String });
dto!(Submission {
    id: String,
    account_id: String,
    operation_id: String,
    draft_id: String,
    draft_revision: u32,
    state: SubmissionState,
    due_at: String,
    timezone: String,
    filing_status: FilingStatus,
    revision: u32
});
dto!(ScheduledMessage { submission: Submission, subject: String, to: Vec<Address> });
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub enum Change {
    Read { value: bool },
    Star { value: bool },
    Archive,
    Trash,
    Spam,
    Move { folder_id: String },
    Category { value: String },
    Snooze { until: String },
}
dto!(Operation {
    id: String,
    account_id: String,
    target_id: String,
    expected_revision: u32,
    change: Change,
    status: OperationStatus,
    revision: u32
});
dto!(InboxFreshness { last_scanned_at: Option<i64>, refreshing: bool });
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub struct AccountCoverage {
    pub account_id: String,
    pub folder_ids: Vec<String>,
    pub state: CoverageState,
    pub error: Option<String>,
    // Only local inbox responses carry scan freshness. Coverage remains an
    // independent assertion about the provider's exposed data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub inbox: Option<InboxFreshness>,
}
dto!(ClassificationCoverage {
    classified: u32,
    total: u32
});
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub struct SearchPage {
    pub rows: Vec<Message>,
    pub continuation: Option<String>,
    pub coverage: Vec<AccountCoverage>,
    pub classification: ClassificationCoverage,
    pub pending: bool,
    pub counts_exact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cache_preview: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub index_page: Option<bool>,
}
dto!(Event {
    account_id: String,
    cursor: u32,
    revision: u32,
    kind: String,
    target_id: String
});
dto!(ModelJob {
    id: String,
    account_id: String,
    message_id: String,
    content_revision: u32,
    model_revision: String,
    kind: JobKind,
    status: JobStatus,
    deadline: String
});
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub struct MessageContent {
    pub message: Message,
    pub text: String,
    pub html: String,
    pub attachments: Vec<Attachment>,
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub cc: Vec<Address>,
    #[serde(default)]
    pub reply_to: Vec<Address>,
    #[serde(default)]
    pub detected_language: Option<String>,
}
dto!(DiscoverAccount { email: String });
dto!(ConversationPage { rows: Vec<Message>, continuation: Option<String>, complete: bool });
dto!(DiscoveredSettings {
    provider: ProviderKind,
    imap_host: String,
    imap_port: u16,
    imap_tls: TlsMode,
    smtp_host: String,
    smtp_port: u16,
    smtp_tls: TlsMode,
    username: String,
    source: String
});
dto!(Session {
    user_id: String,
    csrf_token: String
});
dto!(ApiError {
    code: String,
    message: String
});
dto!(SubmitRequest { operation_id: String, draft_id: String, expected_revision: u32, due_at: Option<String>, timezone: String });
values!(TlsMode { Implicit, Starttls });
// Intentionally not Debug: contains the write-only credential.
#[derive(Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub struct ConnectAccount {
    pub email: String,
    pub display_name: String,
    pub provider: ProviderKind,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: TlsMode,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: TlsMode,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub gmail_oauth: Option<GmailGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub microsoft_oauth: Option<MicrosoftGrant>,
}
// Stored only inside the encrypted account credential, never returned by the API.
#[derive(Clone, Serialize, Deserialize)]
pub struct GmailGrant {
    pub client_id: String,
    pub refresh_token: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MicrosoftGrant {
    pub client_id: String,
    pub tenant: String,
    pub subject: String,
    pub refresh_token: String,
}

// Rules are a closed wire schema: only implemented conditions and actions can
// be accepted. New presets can share matching and durable execution later.
values!(RulePreset { BlockSender });
values!(RuleScope { AllMailboxes });
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub enum RuleCondition {
    SenderIs { email: String },
}
values!(RuleAction {
    Trash,
    SuppressNotifications
});
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub struct CreateRule {
    pub preset: RulePreset,
    pub scope: RuleScope,
    pub condition: RuleCondition,
    pub actions: Vec<RuleAction>,
}
dto!(MailRule {
    id: String, preset: RulePreset, enabled: bool, activated_at: String,
    scope: RuleScope, condition: RuleCondition, actions: Vec<RuleAction>
});
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../packages/api-types/src/generated/")]
pub struct UpdateRule {
    pub enabled: bool,
}
dto!(RuleExecution {
    rule_id: String, message_id: String, operation_id: Option<String>,
    state: String
});
dto!(RuleActivity {
    rule_id: String,
    pending: u32,
    failed: u32
});
