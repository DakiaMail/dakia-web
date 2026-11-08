//! Account-scoped integration adapter. The only entry takes AuthorizedAccount.
#[cfg(test)]
#[path = "archive_tests.rs"]
mod archive_tests;
mod conversation;
mod inbox;
#[cfg(test)]
mod initial_sync_bench;
#[cfg(test)]
#[path = "mail_tests.rs"]
mod integration_tests;
mod reconcile;
#[path = "mail_references.rs"]
mod references;
use super::{ApiFailure, Result};
use axum::http::StatusCode;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use dakia_domain::*;
use dakia_mail_provider as p;
use dakia_storage::{AuthorizedAccount, Error, Store};
use mail_parser::MimeHeaders;

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;
// MIME transfer encoding expands attachments: a valid 25 MiB file can exceed
// 30 MiB on the wire. Keep a finite ceiling below the provider response budget.
const RECEIVED_MESSAGE_LIMIT: u32 = 40 * 1024 * 1024;
type InboxRefreshSlot = Arc<Mutex<Option<std::time::Instant>>>;
type ConversationSlot = (String, Arc<Mutex<conversation::Discovery>>);
type ContentLanes = HashMap<(String, String), std::sync::Weak<Mutex<()>>>;
pub(crate) struct SyncActivity(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for SyncActivity {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}
#[derive(Clone, Default)]
pub struct MailRuntime {
    pub(crate) gmail: Option<crate::gmail_oauth::GmailConfig>,
    languages: Arc<crate::language::Detector>,
    providers: Arc<Mutex<HashMap<String, Arc<dyn p::MailProvider>>>>,
    conversations: Arc<Mutex<HashMap<String, ConversationSlot>>>,
    reconciliation: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    sync_lanes: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    inbox_refreshes: Arc<Mutex<HashMap<String, InboxRefreshSlot>>>,
    content_lanes: Arc<Mutex<ContentLanes>>,
    sync_activity: Arc<Mutex<HashMap<String, Arc<std::sync::atomic::AtomicUsize>>>>,
}
fn provider_reason(detail: &str) -> &'static str {
    match detail {
        "IMAP operation deadline exceeded" => "imap_deadline",
        "IMAP transport interrupted" => "imap_transport",
        "IMAP connection failed" => "imap_connect",
        "IMAP reconnect cooling down" => "imap_backoff",
        "IMAP connection closed before tagged completion" => "imap_eof",
        "IMAP protocol/transport failed" => "imap_protocol_transport",
        "unexpected IMAP completion tag" => "imap_completion_tag",
        "IMAP response budget exceeded" => "imap_response_budget",
        "message exceeds content limit" => "message_size_limit",
        "incomplete or conflicting message body" => "message_body_incomplete",
        "conflicting message sizes" => "message_size_conflict",
        "message size unavailable" => "message_size_unavailable",
        "message body unavailable" => "message_body_unavailable",
        "Gmail identity recovery mismatch" => "reference_recovery_mismatch",
        "ambiguous Gmail identity recovery" => "reference_recovery_ambiguous",
        "Gmail identity recovery unsupported" => "reference_recovery_unsupported",
        "UIDVALIDITY changed" => "reference_epoch_changed",
        "message no longer exists" => "reference_uid_missing",
        "IMAP server closed session" => "imap_bye",
        "source and destination identical" => "mutation_same_folder",
        "spam move source still present; reconcile before retry" => "spam_source_present",
        "spam destination UID unknown; reconcile before retry" => "spam_destination_unknown",
        "spam destination readback failed; reconcile before retry" => "spam_readback_failed",
        "spam destination missing; reconcile before retry" => "spam_destination_missing",
        "copy succeeded but move cleanup incomplete; reconcile before retry" => {
            "move_cleanup_incomplete"
        }
        "IMAP command rejected" => "imap_command_rejected",
        "IMAP provider throttled" => "imap_throttled",
        "IMAP authentication failed" => "imap_authentication",
        "IMAP TLS verification failed" => "imap_tls",
        "archive did not remove Inbox; reconcile before retry" => "archive_membership_present",
        "mutation interrupted; reconcile provider state before retry" => "mutation_interrupted",
        "SMTP deadline exceeded before message body" => "smtp_deadline",
        "SMTP temporary rejection before message body" => "smtp_temporary_rejection",
        "SMTP transport interrupted before message body" => "smtp_transport",
        "SMTP authentication rejected before message body" => "smtp_authentication",
        "SMTP command failed before message body" => "smtp_command_rejected",
        _ => "other",
    }
}

pub(crate) fn provider_error(e: p::ProviderError) -> ApiFailure {
    // Provider response text may contain private data. Log only allowlisted
    // categories, never the raw error, endpoint, username or credentials.
    let (stage, detail) = if let Some(detail) = e.message.strip_prefix("IMAP probe: ") {
        ("imap_probe", detail)
    } else if let Some(detail) = e.message.strip_prefix("SMTP probe: ") {
        ("smtp_probe", detail)
    } else {
        ("mail_operation", e.message.as_str())
    };
    let reason = provider_reason(detail);
    if reason == "imap_backoff" {
        // Suppressed work is not another provider call or failure.
        tracing::info!(
            stage,
            reason,
            operation = "provider_backoff",
            outcome = "skipped",
            "provider_io"
        );
    } else {
        tracing::warn!(stage, reason, kind = ?e.kind, "mail provider request failed");
    }
    if detail == "message exceeds content limit" {
        return ApiFailure(StatusCode::PAYLOAD_TOO_LARGE, "message_too_large");
    }
    ApiFailure(
        match e.kind {
            p::ErrorKind::Authentication => StatusCode::UNAUTHORIZED,
            p::ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
            p::ErrorKind::StaleReference => StatusCode::CONFLICT,
            _ => StatusCode::BAD_GATEWAY,
        },
        match e.kind {
            p::ErrorKind::Authentication if e.message.starts_with("IMAP probe:") => {
                "imap_authentication_rejected"
            }
            p::ErrorKind::Authentication if e.message.starts_with("SMTP probe:") => {
                "smtp_authentication_rejected"
            }
            p::ErrorKind::Authentication => "account_reconnect_required",
            p::ErrorKind::Unsupported => "provider_unsupported",
            p::ErrorKind::Uncertain => "provider_outcome_unknown",
            p::ErrorKind::StaleReference => "provider_reference_changed",
            _ => "provider_unavailable",
        },
    )
}
// Gmail X-GM-LABELS uses system label tokens, not selectable mailbox names.
// imap-proto retains quoted backslash escapes, so accept both wire spellings
// for known system labels only. Preserve custom labels without reinterpretation.
fn mailbox_memberships(folder: &str, labels: Vec<String>, probe: Option<&p::Probe>) -> Vec<String> {
    let mut folders = vec![folder.to_owned()];
    for label in labels {
        let role = label.trim_start_matches('\\');
        let mapped = if role.eq_ignore_ascii_case("Inbox") && label.starts_with('\\') {
            Some("INBOX".to_owned())
        } else if label.starts_with('\\')
            && [
                "Sent",
                "Drafts",
                "Trash",
                "Spam",
                "Junk",
                "Important",
                "Starred",
                "All",
            ]
            .iter()
            .any(|v| role.eq_ignore_ascii_case(v))
        {
            let attribute = if role.eq_ignore_ascii_case("Spam") {
                "Junk"
            } else if role.eq_ignore_ascii_case("Starred") {
                "Flagged"
            } else {
                role
            };
            probe
                .and_then(|p| {
                    p.folders.iter().find(|f| {
                        f.selectable
                            && f.attributes
                                .iter()
                                .any(|a| a.eq_ignore_ascii_case(attribute))
                    })
                })
                .map(|f| f.name.clone())
        } else {
            None
        };
        folders.push(mapped.unwrap_or(label));
    }
    folders.sort();
    folders.dedup();
    folders
}
fn retain_implicit_all_mail(folders: &mut Vec<String>, previous: &[String], probe: &p::Probe) {
    // Gmail omits the implicit All Mail membership from X-GM-LABELS. Do not
    // alternately erase/re-add it as INBOX and All Mail scans arrive: each
    // fictitious change emits an event that triggers another UI search.
    let has_role = |name: &str, role: &str| {
        probe
            .folders
            .iter()
            .any(|f| f.name == name && f.attributes.iter().any(|a| a.eq_ignore_ascii_case(role)))
    };
    if folders
        .iter()
        .any(|f| has_role(f, "Trash") || has_role(f, "Junk"))
    {
        return;
    }
    for old in previous {
        if has_role(old, "All") && !folders.contains(old) {
            folders.push(old.clone());
        }
    }
    folders.sort();
    folders.dedup();
}
fn attachment_bytes(raw: &[u8], part: &mail_parser::MessagePart<'_>) -> Result<Vec<u8>> {
    // contents() is charset-decoded UTF-8 for text/* and can corrupt files.
    // Decode transfer encoding only, keeping original file bytes intact.
    let body = raw
        .get(part.offset_body as usize..part.offset_end as usize)
        .ok_or(ApiFailure(
            StatusCode::BAD_GATEWAY,
            "invalid_attachment_mime",
        ))?;
    let bytes = match part.encoding {
        mail_parser::Encoding::None => Some(body.to_vec()),
        mail_parser::Encoding::Base64 => mail_parser::decoders::base64::base64_decode(body),
        mail_parser::Encoding::QuotedPrintable => {
            mail_parser::decoders::quoted_printable::quoted_printable_decode(body)
        }
    };
    bytes.ok_or(ApiFailure(
        StatusCode::BAD_GATEWAY,
        "invalid_attachment_encoding",
    ))
}
fn security(mode: TlsMode) -> p::Security {
    match mode {
        TlsMode::Implicit => p::Security::Tls,
        TlsMode::Starttls => p::Security::StartTls,
    }
}
pub(crate) fn configuration(a: &AuthorizedAccount, input: &ConnectAccount) -> p::Config {
    p::Config {
        account_id: a.id().into(),
        primary_address: input.email.clone(),
        kind: match input.provider {
            ProviderKind::Gmail => p::ProviderKind::Gmail,
            ProviderKind::Imap => p::ProviderKind::Imap,
        },
        imap: p::Endpoint {
            host: input.imap_host.clone(),
            port: input.imap_port,
            security: security(input.imap_tls.clone()),
        },
        smtp: p::Endpoint {
            host: input.smtp_host.clone(),
            port: input.smtp_port,
            security: security(input.smtp_tls.clone()),
        },
        timeout: Duration::from_secs(20),
        full_imap_exposure_confirmed: false,
        folders: p::FolderMapping::default(),
    }
}
#[derive(Serialize, Deserialize)]
struct PartRef {
    message_id: String,
    index: usize,
}
fn arrival_time(value: &str) -> Result<String> {
    chrono::DateTime::parse_from_str(value.trim(), "%d-%b-%Y %H:%M:%S %z")
        .map(|date| {
            date.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
        .map_err(|_| ApiFailure(StatusCode::BAD_GATEWAY, "invalid_provider_arrival_time"))
}
impl MailRuntime {
    pub fn with_gmail(mut self, config: Option<crate::gmail_oauth::GmailConfig>) -> Self {
        self.gmail = config;
        self
    }
    async fn content_lane(&self, a: &AuthorizedAccount, id: &str) -> Arc<Mutex<()>> {
        let mut lanes = self.content_lanes.lock().await;
        lanes.retain(|_, lane| lane.strong_count() > 0);
        let lane = lanes.entry((a.id().into(), id.into())).or_default();
        if let Some(existing) = lane.upgrade() {
            return existing;
        }
        let new = Arc::new(Mutex::new(()));
        *lane = Arc::downgrade(&new);
        new
    }
    // Cover provider I/O AND persistence. The provider's connection lane ends
    // before page ingestion, otherwise a pre-MOVE scan can overwrite its result.
    pub(crate) async fn reconciliation_lane(&self, a: &AuthorizedAccount) -> Arc<Mutex<()>> {
        self.reconciliation
            .lock()
            .await
            .entry(a.id().into())
            .or_default()
            .clone()
    }

    pub(crate) async fn remove_account(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        confirmation: &str,
    ) -> Result<()> {
        let lane = self.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        let mut providers = self.providers.lock().await;
        store.remove_account(a, confirmation).await?;
        if let Some(provider) = providers.remove(a.id()) {
            provider.retire();
        }
        self.inbox_refreshes.lock().await.remove(a.id());
        self.sync_lanes.lock().await.remove(a.id());
        Ok(())
    }
    pub(crate) async fn settings(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
    ) -> Result<MailboxSettings> {
        Ok(store
            .provider_document(a, "mailbox-settings")
            .await?
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(Error::from)?
            .unwrap_or_default())
    }
    pub(crate) async fn configure(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        settings: MailboxSettings,
    ) -> Result<MailboxSettings> {
        if settings.received_cache_budget < 1024 * 1024 {
            return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_cache_budget"));
        }
        let folders = self.folders(store, a).await?;
        for name in [
            &settings.sent_folder,
            &settings.archive_folder,
            &settings.trash_folder,
        ]
        .into_iter()
        .flatten()
        {
            if !folders.iter().any(|f| &f.name == name) {
                return Err(ApiFailure(
                    StatusCode::BAD_REQUEST,
                    "invalid_folder_mapping",
                ));
            }
        }
        let lane = self.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        let mut providers = self.providers.lock().await;
        store
            .put_provider_document(
                a,
                "mailbox-settings",
                &serde_json::to_string(&settings).map_err(Error::from)?,
            )
            .await?;
        if let Some(provider) = providers.remove(a.id()) {
            provider.retire();
        }
        Ok(settings)
    }
    pub(crate) async fn reconnect(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        input: &ConnectAccount,
    ) -> Result<Account> {
        // Serialize replacement against provider construction; never reuse old credentials.
        let lane = self.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        let mut providers = self.providers.lock().await;
        let account = store.reconnect(a, input).await?;
        if let Some(provider) = providers.remove(a.id()) {
            provider.retire();
        }
        Ok(account)
    }
    pub(crate) async fn folders(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
    ) -> Result<Vec<p::Folder>> {
        let folders = self
            .provider(store, a)
            .await?
            .folders()
            .await
            .map_err(provider_error)?;
        // Refresh discovered roles without turning an IMAP-only listing into
        // an account readiness/SMTP probe. This also replaces legacy Debug
        // spellings of extension roles such as Gmail's Important mailbox.
        if let Some(raw) = store.provider_document(a, "probe").await? {
            if let Ok(mut probe) = serde_json::from_str::<p::Probe>(&raw) {
                probe.folders = folders.clone();
                store
                    .put_provider_document(
                        a,
                        "probe",
                        &serde_json::to_string(&probe).map_err(Error::from)?,
                    )
                    .await?;
            }
        }
        let folders: Vec<p::Folder> = folders.into_iter().filter(|f| f.selectable).collect();
        store
            .put_provider_document(
                a,
                "thread:folders",
                &serde_json::json!({
                    "names": folders.iter().map(|f| &f.name).collect::<Vec<_>>(),
                    "at": chrono::Utc::now().timestamp()
                })
                .to_string(),
            )
            .await?;
        Ok(folders)
    }
    pub(crate) async fn provider(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
    ) -> Result<Arc<dyn p::MailProvider>> {
        let mut providers = self.providers.lock().await;
        if let Some(p) = providers.get(a.id()) {
            return Ok(p.clone());
        }
        let input = store.credential(a).await?;
        let mut config = configuration(a, &input);
        let settings = self.settings(store, a).await?;
        config.full_imap_exposure_confirmed = settings.full_imap_exposure_confirmed;
        config.folders = p::FolderMapping {
            sent: settings.sent_folder,
            archive: settings.archive_folder,
            trash: settings.trash_folder,
        };
        let oauth = if let Some(grant) = input.gmail_oauth {
            let config = self
                .gmail
                .clone()
                .filter(|c| c.client_id == grant.client_id)
                .ok_or(ApiFailure(
                    StatusCode::UNAUTHORIZED,
                    "account_reconnect_required",
                ))?;
            Some(Arc::new(crate::gmail_oauth::GoogleToken::new(
                config, grant,
            )))
        } else {
            None
        };
        let provider = p::AccountProvider::new(
            config,
            p::Credentials {
                username: input.username,
                password: input.password,
            },
        )
        .map_err(provider_error)?;
        let provider: Arc<dyn p::MailProvider> = Arc::new(if let Some(source) = oauth {
            provider.with_oauth(source)
        } else {
            provider
        });
        if provider.account_id() != a.id() {
            return Err(ApiFailure(StatusCode::FORBIDDEN, "account_mismatch"));
        }
        providers.insert(a.id().into(), provider.clone());
        Ok(provider)
    }
    pub(crate) async fn probe(&self, store: &Store, a: &AuthorizedAccount) -> Result<Account> {
        let p = self.provider(store, a).await?;
        match p.probe().await {
            Ok(probe) => {
                store
                    .put_provider_document(
                        a,
                        "probe",
                        &serde_json::to_string(&probe).map_err(Error::from)?,
                    )
                    .await?;
                Ok(store.account_state(a, AccountState::Ready).await?)
            }
            Err(e) => {
                let state = if e.kind == p::ErrorKind::Authentication {
                    AccountState::Reconnect
                } else {
                    AccountState::Error
                };
                store.account_state(a, state).await?;
                Err(provider_error(e))
            }
        }
    }
    pub(crate) async fn page(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        query: p::Search,
        continuation: Option<&str>,
    ) -> Result<SearchPage> {
        self.page_limit(store, a, folder, query, continuation, 50)
            .await
    }
    #[cfg(test)]
    pub(crate) async fn snapshot_page(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        continuation: Option<&str>,
    ) -> Result<SearchPage> {
        self.reconcile_page(store, a, folder, continuation, None)
            .await
    }
    pub(crate) async fn record_snapshot_page(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        generation: i64,
        continuation: Option<&str>,
    ) -> Result<()> {
        self.reconcile_page(store, a, folder, continuation, Some(generation))
            .await?;
        Ok(())
    }
    #[tracing::instrument(target = "dakia_server::diagnostics", skip_all, fields(account_key = crate::diagnostics::correlation(a.id()), folder_key = crate::diagnostics::correlation(folder)))]
    async fn page_limit(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        query: p::Search,
        continuation: Option<&str>,
        limit: usize,
    ) -> Result<SearchPage> {
        let lane = self.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        let provider = self.provider(store, a).await?;
        let token: Option<p::PageToken> = continuation
            .map(|s| {
                URL_SAFE_NO_PAD
                    .decode(s)
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .ok_or(ApiFailure(StatusCode::BAD_REQUEST, "invalid_continuation"))
            })
            .transpose()?;
        let _activity = self.begin_sync(a).await;
        let page = provider
            .scan(folder, &query, token.as_ref(), limit)
            .await
            .map_err(provider_error)?;
        let rows = self
            .ingest_metadata(store, a, folder, page.uidvalidity, page.rows)
            .await?;
        let continuation = page
            .next
            .map(|t| serde_json::to_vec(&t).map(|b| URL_SAFE_NO_PAD.encode(b)))
            .transpose()
            .map_err(Error::from)?;
        let total = rows.len() as u32;
        Ok(SearchPage {
            rows,
            continuation,
            coverage: vec![AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec![folder.into()],
                inbox: None,
                state: if page.full_coverage {
                    CoverageState::Complete
                } else {
                    CoverageState::Partial
                },
                error: None,
            }],
            classification: ClassificationCoverage {
                classified: 0,
                total,
            },
            pending: false,
            counts_exact: false,
            cache_preview: None,
            index_page: None,
        })
    }

    async fn ingest_metadata(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        validity: u32,
        metadata: Vec<p::Metadata>,
    ) -> Result<Vec<Message>> {
        self.ingest_metadata_at(store, a, folder, validity, metadata, 0)
            .await
    }
    async fn ingest_metadata_at(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        validity: u32,
        metadata: Vec<p::Metadata>,
        observed_after: i64,
    ) -> Result<Vec<Message>> {
        if metadata.len() > 200
            || metadata.iter().any(|m| {
                m.reference.account_id != a.id()
                    || m.reference.folder != folder
                    || m.reference.uidvalidity != validity
                    || m.reference.uid == 0
            })
        {
            return Err(ApiFailure(
                StatusCode::BAD_GATEWAY,
                "invalid_provider_metadata",
            ));
        }
        // A changed UIDVALIDITY invalidates old IMAP identities immediately,
        // even before a paginated full reconciliation has finished pruning.
        let epoch_key = format!("thread:epoch:{folder}");
        let epoch = validity.to_string();
        if store
            .provider_document(a, &epoch_key)
            .await?
            .is_some_and(|old| old != epoch)
        {
            store.reset_scan(a, folder).await?;
        }
        store.put_provider_document(a, &epoch_key, &epoch).await?;
        let probe: Option<p::Probe> = store
            .provider_document(a, "probe")
            .await?
            .and_then(|raw| serde_json::from_str(&raw).ok());
        let sent = self.settings(store, a).await?.sent_folder.as_deref() == Some(folder)
            || probe.as_ref().is_some_and(|p| {
                p.folders.iter().any(|f| {
                    f.name == folder
                        && f.attributes
                            .iter()
                            .any(|v| v.trim_start_matches('\\').eq_ignore_ascii_case("Sent"))
                })
            });
        // Validate the whole provider page before any persistence.
        let provider_ids: Vec<String> = metadata
            .iter()
            .map(|metadata| {
                if metadata.reference.account_id != a.id()
                    || match &metadata.identity {
                        p::Identity::Gmail { account_id, .. } => account_id != a.id(),
                        p::Identity::Imap(r) => r.account_id != a.id(),
                    }
                {
                    return Err(ApiFailure(StatusCode::FORBIDDEN, "account_mismatch"));
                }
                Ok(match &metadata.identity {
                    p::Identity::Gmail { message_id, .. } => format!("gmail:{message_id}"),
                    p::Identity::Imap(r) => serde_json::to_string(r).map_err(Error::from)?,
                })
            })
            .collect::<Result<_>>()?;
        let ids = store.message_ids(a, &provider_ids).await?;
        let mut messages = vec![];
        let mut documents = vec![];
        let mut recipients = vec![];
        let mut rows = vec![];
        for ((metadata, provider_id), id) in metadata.into_iter().zip(provider_ids).zip(ids) {
            let parsed = mail_parser::MessageParser::default()
                .parse(&metadata.headers)
                .ok_or(ApiFailure(
                    StatusCode::BAD_GATEWAY,
                    "invalid_message_headers",
                ))?;
            let address = |v: &mail_parser::Addr<'_>| Address {
                email: v.address().unwrap_or("").into(),
                name: v.name().map(str::to_owned),
            };
            let root = parsed
                .references()
                .as_text_list()
                .and_then(|v| v.first())
                .map(|v| v.as_ref())
                .or_else(|| parsed.in_reply_to().as_text())
                .or_else(|| parsed.message_id())
                .unwrap_or(&id);
            let thread = metadata
                .gmail_thread_id
                .map(|v| format!("gmail-thread:{v}"))
                .unwrap_or_else(|| format!("references:{root}"));
            let mut folder_ids = mailbox_memberships(folder, metadata.labels, probe.as_ref());
            if matches!(metadata.identity, p::Identity::Gmail { .. }) {
                if let Some(probe) = &probe {
                    match store.message(a, &id).await {
                        Ok(old) => {
                            retain_implicit_all_mail(&mut folder_ids, &old.folder_ids, probe)
                        }
                        Err(Error::NotFound) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            let m = Message {
                id: id.clone(),
                account_id: a.id().into(),
                thread_id: thread,
                provider_id,
                folder_ids,
                sender: parsed
                    .from()
                    .and_then(|v| v.first())
                    .map(address)
                    .unwrap_or(Address {
                        email: String::new(),
                        name: None,
                    }),
                to: parsed
                    .to()
                    .map(|v| v.iter().map(address).collect())
                    .unwrap_or_default(),
                subject: parsed.subject().unwrap_or("").into(),
                preview: String::new(),
                received_at: arrival_time(&metadata.internal_date)?,
                unread: !metadata.seen,
                starred: metadata.starred,
                category: None,
                category_status: CategoryStatus::Pending,
                revision: 0,
            };
            if sent {
                let addresses: Vec<Address> = parsed
                    .to()
                    .into_iter()
                    .chain(parsed.cc())
                    .chain(parsed.bcc())
                    .flat_map(|v| v.iter())
                    .map(address)
                    .collect();
                recipients.push((format!("sent:{id}"), addresses, m.received_at.clone()));
            }
            let reference = serde_json::to_string(&metadata.reference).map_err(Error::from)?;
            documents.push((
                format!("__content_observation:{id}"),
                observed_after.to_string(),
            ));
            documents.push((format!("reference:{id}"), reference.clone()));
            documents.push((reconcile::reference_key(&metadata.reference), id.clone()));
            documents.push((
                format!("member-reference:{id}:{}", metadata.reference.folder),
                reference,
            ));
            documents.push(conversation::remembered_document(
                &id,
                &metadata.headers,
                metadata.gmail_thread_id,
                metadata.size,
            )?);
            messages.push(m);
        }
        let persisted = store.ingest_messages(a, messages, &documents).await?;
        for (key, addresses, received_at) in recipients {
            store
                .learn_recipients(a, &key, &addresses, &received_at)
                .await?;
        }
        for message in persisted {
            let id = message.id.clone();
            if !rows.iter().any(|m: &Message| m.id == id) {
                let message = store.visible_message(a, message).await?;
                if !folder.eq_ignore_ascii_case("INBOX")
                    || message
                        .folder_ids
                        .iter()
                        .any(|f| f.eq_ignore_ascii_case("INBOX"))
                {
                    rows.push(message)
                }
            }
        }
        Ok(rows)
    }
    #[tracing::instrument(target = "dakia_server::diagnostics", skip_all, fields(account_key = crate::diagnostics::correlation(a.id()), message_key = crate::diagnostics::correlation(id)))]
    pub(crate) async fn raw(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<Vec<u8>> {
        self.raw_with_limit(store, a, id, RECEIVED_MESSAGE_LIMIT)
            .await
    }
    #[tracing::instrument(target = "dakia_server::diagnostics", skip_all, fields(account_key = crate::diagnostics::correlation(a.id()), message_key = crate::diagnostics::correlation(id)))]
    pub(crate) async fn raw_with_limit(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
        limit: u32,
    ) -> Result<Vec<u8>> {
        if let Some(raw) = store.cached_raw(a, id).await? {
            tracing::info!(target: "dakia_server::diagnostics", operation = "content_cache", outcome = "hit", "mail_work");
            if raw.len() > limit as usize {
                return Err(ApiFailure(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "message_too_large",
                ));
            }
            return Ok(raw);
        }
        let reference = store
            .provider_document(a, &format!("reference:{id}"))
            .await?
            .ok_or(Error::NotFound)?;
        let reference: p::MessageRef = serde_json::from_str(&reference).map_err(Error::from)?;
        let message = store.message(a, id).await?;
        if message.provider_id.starts_with("gmail:") {
            return self.gmail_raw(store, a, &message, reference, limit).await;
        }
        let provider = self.provider(store, a).await?;
        let first_error = match provider.fetch(&reference, limit).await {
            Ok(raw) => return Ok(raw),
            Err(error) => error,
        };
        // Gmail's stable identity can survive loss of one label/UID. Only use
        // references actually observed for this same account/message, never
        // substitute a generic IMAP UID or retry an authentication/network error.
        let message = store.message(a, id).await?;
        if first_error.kind == p::ErrorKind::StaleReference
            && reference.account_id == a.id()
            && serde_json::from_str::<p::MessageRef>(&message.provider_id)
                .is_ok_and(|identity| identity == reference)
        {
            let invalidated = store
                .invalidate_stale_reference(a, id, &reference.folder, &message.provider_id)
                .await?;
            tracing::info!(target: "dakia_server::diagnostics", operation = "stale_reference_reconcile", outcome = if invalidated { "scheduled" } else { "already_pending" }, "mail_work");
        }
        Err(provider_error(first_error))
    }
    pub(crate) async fn content(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<MessageContent> {
        let lane = self.content_lane(a, id).await;
        let _guard = lane.lock().await;
        if let Some(mut c) = store.cached_content(a, id).await? {
            // Re-evaluate legacy cached documents too, without fetching or mutating mail.
            c.detected_language = self.languages.detect(a.id(), id, &c.text, &c.html);
            return Ok(c);
        }
        let raw = self.raw(store, a, id).await?;
        self.cache_parsed_content(store, a, id, &raw).await
    }
    async fn cache_parsed_content(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
        raw: &[u8],
    ) -> Result<MessageContent> {
        let message = store.message(a, id).await?;
        let parsed = mail_parser::MessageParser::default()
            .parse(raw)
            .ok_or(ApiFailure(StatusCode::BAD_GATEWAY, "invalid_message_mime"))?;
        let mut attachments = vec![];
        for (index, part) in parsed.attachments().enumerate() {
            let key = format!("part-id:{id}:{index}");
            let part_id = match store.provider_document(a, &key).await? {
                Some(v) => v,
                None => {
                    let v = uuid::Uuid::new_v4().to_string();
                    store.put_provider_document(a, &key, &v).await?;
                    v
                }
            };
            store
                .put_provider_document(
                    a,
                    &format!("received-attachment:{part_id}"),
                    &serde_json::to_string(&PartRef {
                        message_id: id.into(),
                        index,
                    })
                    .map_err(Error::from)?,
                )
                .await?;
            let content_type = part
                .content_type()
                .map(|c| {
                    format!(
                        "{}/{}",
                        c.c_type,
                        c.c_subtype.as_deref().unwrap_or("octet-stream")
                    )
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            attachments.push(Attachment {
                id: part_id,
                filename: part.attachment_name().unwrap_or("attachment.bin").into(),
                content_type,
                size_bytes: attachment_bytes(raw, part)?.len() as u32,
            });
        }
        let mut content = MessageContent {
            reply_to: parsed
                .reply_to()
                .map(|v| {
                    v.iter()
                        .map(|a| Address {
                            email: a.address().unwrap_or("").into(),
                            name: a.name().map(str::to_owned),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            detected_language: None,
            message,
            message_id: conversation::message_id(&parsed),
            references: conversation::references(&parsed),
            cc: parsed
                .cc()
                .map(|v| {
                    v.iter()
                        .map(|a| Address {
                            email: a.address().unwrap_or("").into(),
                            name: a.name().map(str::to_owned),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            text: parsed
                .body_text(0)
                .map(|v| v.into_owned())
                .unwrap_or_default(),
            html: parsed
                .body_html(0)
                .map(|v| v.into_owned())
                .unwrap_or_default(),
            attachments,
        };

        content.detected_language = self
            .languages
            .detect(a.id(), id, &content.text, &content.html);
        crate::unsubscribe::remember(store, a, id, raw).await?;
        store
            .cache_content_with_raw(
                a,
                &content,
                Some(raw),
                u64::from(self.settings(store, a).await?.received_cache_budget),
            )
            .await?;
        Ok(content)
    }
    #[cfg(test)]
    pub(crate) async fn received_attachment(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<Vec<u8>> {
        Ok(self.received_part(store, a, id).await?.1)
    }
    pub(crate) async fn pin_received_attachment(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<Attachment> {
        if let Ok((metadata, _)) = store.attachment(a, id).await {
            return Ok(metadata);
        }
        let (metadata, bytes) = self.received_part(store, a, id).await?;
        Ok(store
            .pin_attachment(a, id, metadata.filename, metadata.content_type, &bytes)
            .await?)
    }
    pub(crate) async fn received_part(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<(Attachment, Vec<u8>)> {
        let part = store
            .provider_document(a, &format!("received-attachment:{id}"))
            .await?
            .ok_or(Error::NotFound)?;
        let part: PartRef = serde_json::from_str(&part).map_err(Error::from)?;
        let lane = self.content_lane(a, &part.message_id).await;
        let _guard = lane.lock().await;
        let raw = match store.cached_raw(a, &part.message_id).await? {
            Some(raw) => raw,
            None => {
                let raw = self.raw(store, a, &part.message_id).await?;
                self.cache_parsed_content(store, a, &part.message_id, &raw)
                    .await?;
                raw
            }
        };
        let parsed = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or(ApiFailure(StatusCode::BAD_GATEWAY, "invalid_message_mime"))?;
        let part = parsed
            .attachments()
            .nth(part.index)
            .ok_or(Error::NotFound)?;
        let bytes = attachment_bytes(&raw, part)?;
        let metadata = Attachment {
            id: id.to_owned(),
            filename: part.attachment_name().unwrap_or("attachment.bin").into(),
            content_type: part
                .content_type()
                .map(|c| {
                    format!(
                        "{}/{}",
                        c.c_type,
                        c.c_subtype.as_deref().unwrap_or("octet-stream")
                    )
                })
                .unwrap_or_else(|| "application/octet-stream".into()),
            size_bytes: bytes.len() as u32,
        };
        Ok((metadata, bytes))
    }
}

#[cfg(test)]
mod reason_tests {
    #[test]
    fn body_failures_have_static_private_reason_codes() {
        assert_eq!(
            super::provider_reason("incomplete or conflicting message body"),
            "message_body_incomplete"
        );
        assert_eq!(
            super::provider_reason("conflicting message sizes"),
            "message_size_conflict"
        );
        assert_eq!(
            super::provider_reason("message size unavailable"),
            "message_size_unavailable"
        );
        assert_eq!(
            super::provider_reason("private@example.invalid secret server response"),
            "other"
        );
    }
}
