use crate::{imap::ImapProvider, mime::PreparedMessage, smtp::SmtpProvider, *};
use async_trait::async_trait;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
type FolderCache = Option<(Instant, Result<Vec<Folder>>)>;
#[path = "scan_cache.rs"]
mod scan_cache;
use scan_cache::ScanCache;

/// Integration implements this with an atomic persisted claimed -> submitting
/// transition. A stopped process in submitting is delivery_unknown on recovery.
#[async_trait]
pub trait SubmissionGate: Send + Sync {
    async fn begin_data(&self) -> Result<()>;
}
/// Durable evidence is scoped to the authorized account, never RFC Message-ID.
#[async_trait]
pub trait MetadataLookup: Send + Sync {
    async fn identity_known(&self, identity: &Identity) -> Result<bool>;
    async fn reference_known(&self, reference: &MessageRef) -> Result<bool>;
}
/// Account-scoped protocol contract. The server MUST resolve AuthorizedAccount
/// before obtaining this object, then compare account_id() with capability.id().
/// No caller-supplied paths, arbitrary raw commands or permanent-delete API.
#[async_trait]
pub trait MailProvider: Send + Sync {
    fn account_id(&self) -> &str;
    fn retire(&self) {}
    fn supports_head(&self) -> bool {
        false
    }
    async fn probe(&self) -> Result<Probe>;
    /// Folder discovery must not depend on outgoing SMTP availability.
    async fn folders(&self) -> Result<Vec<Folder>> {
        Ok(self.probe().await?.folders)
    }
    async fn scan(
        &self,
        folder: &str,
        query: &Search,
        continuation: Option<&PageToken>,
        limit: usize,
    ) -> Result<Page>;
    async fn reconcile(
        &self,
        folder: &str,
        continuation: Option<&PageToken>,
        limit: usize,
        _known: &dyn MetadataLookup,
        _since: Option<&ModseqCheckpoint>,
    ) -> Result<ReconcilePage> {
        let page = self
            .scan(folder, &Search::default(), continuation, limit)
            .await?;
        Ok(ReconcilePage {
            rows: page.rows.into_iter().map(MetadataChange::New).collect(),
            next: page.next,
            uidvalidity: page.uidvalidity,
            full_coverage: page.full_coverage,
            checkpoint: None,
        })
    }
    /// True only when the provider can prove the committed snapshot is current.
    /// Unsupported providers keep the ordinary full traversal fallback.
    async fn snapshot_unchanged(&self, _checkpoint: &ModseqCheckpoint) -> Result<bool> {
        Ok(false)
    }
    /// Read-only recovery after an uncertain write. Never repeats the mutation.
    async fn verify_mutation(
        &self,
        _reference: &MessageRef,
        _identity: &Identity,
        _change: &Mutation,
    ) -> Result<Option<MutationResult>> {
        Ok(None)
    }
    /// Incremental arrivals only, not interchangeable with a snapshot Page.
    async fn head(&self, _folder: &str, _after: Option<&HeadCheckpoint>) -> Result<HeadPage> {
        Err(error(
            ErrorKind::Unsupported,
            "new-mail head checks unsupported",
        ))
    }
    async fn search(
        &self,
        folder: &str,
        query: &Search,
        continuation: Option<&PageToken>,
        limit: usize,
    ) -> Result<Page>;
    async fn wait_for_change(&self, folder: &str, max_wait: std::time::Duration) -> Result<bool>;
    async fn fetch(&self, reference: &MessageRef, max_bytes: u32) -> Result<Vec<u8>>;
    /// At most 50 small messages in one folder/epoch, in input order.
    /// Unsupported callers retain their existing single-message semantics.
    async fn fetch_batch(
        &self,
        _references: &[MessageRef],
        _max_bytes: u32,
    ) -> Result<Vec<Vec<u8>>> {
        Err(error(ErrorKind::Unsupported, "body batching unsupported"))
    }
    /// Bounded, read-only stable Gmail identity discovery. Not a text search.
    async fn resolve_gmail_message(
        &self,
        _message_id: u64,
        _folders: &[String],
    ) -> Result<Option<Metadata>> {
        Err(error(
            ErrorKind::Unsupported,
            "Gmail identity recovery unsupported",
        ))
    }
    async fn mutate(&self, reference: &MessageRef, change: Mutation) -> Result<MutationResult>;
    async fn submit(
        &self,
        prepared: &PreparedMessage,
        gate: &dyn SubmissionGate,
    ) -> SubmissionOutcome;
    async fn reconcile_sent(&self, message_id: &str, raw: &[u8]) -> Result<SentOutcome>;
}
// A dropped pooled socket is recoverable for reads. Never retry deadlines,
// throttling, authentication, stale identities or any write through this helper.
async fn retry_read<T, F, Fut>(mut load: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match load().await {
        Err(e)
            if e.kind == ErrorKind::Retryable
                && matches!(
                    e.message.as_str(),
                    "IMAP connection closed before tagged completion"
                        | "IMAP transport interrupted"
                        | "IMAP protocol/transport failed"
                ) =>
        {
            tracing::info!(target: "dakia_server::provider_io", operation = "read_reconnect", "retrying interrupted read once");
            load().await
        }
        result => result,
    }
}
pub struct AccountProvider {
    imap: ImapProvider,
    smtp: SmtpProvider,
    folder_cache: Mutex<FolderCache>,
    scan_cache: Mutex<ScanCache>,
}
impl AccountProvider {
    /// Add an explicit trust root for private mail servers and disposable fixtures.
    pub fn with_imap_root(mut self, root: native_tls::Certificate) -> Result<Self> {
        self.imap = self.imap.with_root(root)?;
        Ok(self)
    }

    async fn lock_scan_cache(
        &self,
        operation: &'static str,
    ) -> Result<tokio::sync::MutexGuard<'_, ScanCache>> {
        let started = Instant::now();
        let result = tokio::time::timeout(self.imap.config.timeout, self.scan_cache.lock()).await;
        tracing::info!(target: "dakia_server::provider_io", operation,
            stage = "scan_cache_wait", wait_ms = started.elapsed().as_millis() as u64,
            deadline_ms = self.imap.config.timeout.as_millis() as u64,
            acquired = result.is_ok(), "mail provider cache lane");
        result.map_err(|_| error(ErrorKind::Retryable, "IMAP operation deadline exceeded"))
    }
    pub fn new(config: Config, credentials: Credentials) -> Result<Self> {
        let smtp_credentials = Credentials {
            username: credentials.username.clone(),
            password: credentials.password.clone(),
        };
        Ok(Self {
            imap: ImapProvider::new(config.clone(), credentials)?,
            smtp: SmtpProvider::new(config, smtp_credentials)?,
            folder_cache: Mutex::new(None),
            scan_cache: Mutex::new(ScanCache::default()),
        })
    }
}
#[async_trait]
impl MailProvider for AccountProvider {
    fn retire(&self) {
        self.imap.retire();
    }
    fn supports_head(&self) -> bool {
        true
    }
    fn account_id(&self) -> &str {
        &self.imap.config.account_id
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), request_kind = "probe"))]
    async fn probe(&self) -> Result<Probe> {
        tracing::info!(target: "dakia_server::provider_io", operation = "imap_probe", "provider_io");
        let result = self.imap.probe().await.map_err(|mut error| {
            error.message = format!("IMAP probe: {}", error.message);
            error
        })?;
        tracing::info!(target: "dakia_server::provider_io", operation = "smtp_probe", "provider_io");
        self.smtp.probe().await.map_err(|mut error| {
            error.message = format!("SMTP probe: {}", error.message);
            error
        })?;
        Ok(result)
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), purpose = "folder_discovery", request_kind = "folders"))]
    async fn folders(&self) -> Result<Vec<Folder>> {
        // Single-flight discovery across concurrent search/UI/sync requests.
        // Briefly cache failures too, rather than queueing another full probe
        // for every waiter during a provider outage. Reconnect replaces self.
        let mut cached = tokio::time::timeout(self.imap.config.timeout, self.folder_cache.lock())
            .await
            .map_err(|_| error(ErrorKind::Retryable, "IMAP operation deadline exceeded"))?;
        if let Some((at, result)) = cached.as_ref() {
            let ttl = if result.is_ok() { 60 } else { 5 };
            if at.elapsed() < Duration::from_secs(ttl) {
                return result.clone();
            }
        }
        tracing::info!(target: "dakia_server::provider_io", operation = "folders", "provider_io");
        let result = retry_read(|| self.imap.probe()).await.map(|p| p.folders);
        *cached = Some((Instant::now(), result.clone()));
        result
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), folder_key = %crate::telemetry::key(folder), purpose = if query.starred == Some(true) { "starred" } else { "search" }, request_kind = "scan"))]
    async fn scan(
        &self,
        folder: &str,
        query: &Search,
        continuation: Option<&PageToken>,
        limit: usize,
    ) -> Result<Page> {
        let key = serde_json::to_string(&(folder, query, continuation, limit)).map_err(|_| {
            ProviderError {
                kind: ErrorKind::InvalidInput,
                message: "invalid scan key".into(),
            }
        })?;
        self.lock_scan_cache("scan").await?
            .get(key, || async {
                tracing::info!(target: "dakia_server::provider_io", operation = "scan", "provider_io");
                retry_read(|| self.imap.scan(folder, query, continuation, limit)).await
            })
            .await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), folder_key = %crate::telemetry::key(folder), purpose = if query.starred == Some(true) { "starred" } else { "search" }, request_kind = "search"))]
    async fn search(
        &self,
        folder: &str,
        query: &Search,
        continuation: Option<&PageToken>,
        limit: usize,
    ) -> Result<Page> {
        self.scan(folder, query, continuation, limit).await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), folder_key = %crate::telemetry::key(folder), purpose = "metadata", request_kind = "reconcile"))]
    async fn reconcile(
        &self,
        folder: &str,
        continuation: Option<&PageToken>,
        limit: usize,
        known: &dyn MetadataLookup,
        since: Option<&ModseqCheckpoint>,
    ) -> Result<ReconcilePage> {
        self.lock_scan_cache("reconcile")
            .await?
            .uncached(|| {
                retry_read(|| {
                    self.imap
                        .reconcile(folder, continuation, limit, known, since)
                })
            })
            .await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), request_kind = "snapshot_unchanged"))]
    async fn snapshot_unchanged(&self, checkpoint: &ModseqCheckpoint) -> Result<bool> {
        self.lock_scan_cache("snapshot_check")
            .await?
            .uncached(|| retry_read(|| self.imap.snapshot_unchanged(checkpoint)))
            .await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), request_kind = "verify_mutation"))]
    async fn verify_mutation(
        &self,
        reference: &MessageRef,
        identity: &Identity,
        change: &Mutation,
    ) -> Result<Option<MutationResult>> {
        self.imap.verify_mutation(reference, identity, change).await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), folder_key = %crate::telemetry::key(folder), purpose = "arrival_check", request_kind = "head"))]
    async fn head(&self, folder: &str, after: Option<&HeadCheckpoint>) -> Result<HeadPage> {
        // Revalidate epoch on the wire even after a checkpoint-persistence crash.
        self.lock_scan_cache("head")
            .await?
            .uncached(|| retry_read(|| self.imap.head(folder, after)))
            .await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), folder_key = %crate::telemetry::key(folder), purpose = "arrival_listener", request_kind = "wait_for_change"))]
    async fn wait_for_change(&self, folder: &str, max_wait: std::time::Duration) -> Result<bool> {
        tracing::info!(target: "dakia_server::provider_io", operation = "idle", "provider_io");
        let changed = self.imap.wait_for_change(folder, max_wait).await?;
        if changed {
            self.scan_cache.lock().await.clear();
        }
        Ok(changed)
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), request_kind = "fetch"))]
    async fn fetch_batch(&self, references: &[MessageRef], max_bytes: u32) -> Result<Vec<Vec<u8>>> {
        self.imap.fetch_batch(references, max_bytes).await
    }
    async fn fetch(&self, reference: &MessageRef, max_bytes: u32) -> Result<Vec<u8>> {
        tracing::info!(target: "dakia_server::provider_io", operation = "fetch", "provider_io");
        retry_read(|| self.imap.fetch(reference, max_bytes)).await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), purpose="reference_recovery", request_kind = "resolve_gmail"))]
    async fn resolve_gmail_message(
        &self,
        message_id: u64,
        folders: &[String],
    ) -> Result<Option<Metadata>> {
        self.imap.resolve_gmail_message(message_id, folders).await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), request_kind = "mutate"))]
    async fn mutate(&self, reference: &MessageRef, change: Mutation) -> Result<MutationResult> {
        // Serialize invalidation with in-flight scans, including uncertain
        // outcomes. No pre-mutation page may repopulate the cache afterwards.
        let mut cache = self.lock_scan_cache("mutate").await?;
        cache.clear();
        tracing::info!(target: "dakia_server::provider_io", operation = "mutate", "provider_io");
        let result = self.imap.mutate(reference, change).await;
        cache.clear();
        result
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), request_kind = "submit"))]
    async fn submit(
        &self,
        prepared: &PreparedMessage,
        gate: &dyn SubmissionGate,
    ) -> SubmissionOutcome {
        tracing::info!(target: "dakia_server::provider_io", operation = "submit", "provider_io");
        self.smtp.submit(prepared, || gate.begin_data()).await
    }
    #[tracing::instrument(target = "dakia_server::provider_io", skip_all, fields(account_key = %crate::telemetry::key(&self.imap.config.account_id), request_key = %crate::telemetry::request_key(), request_kind = "reconcile_sent"))]
    async fn reconcile_sent(&self, message_id: &str, raw: &[u8]) -> Result<SentOutcome> {
        let mut cache = tokio::time::timeout(self.imap.config.timeout, self.scan_cache.lock())
            .await
            .map_err(|_| error(ErrorKind::Retryable, "IMAP operation deadline exceeded"))?;
        cache.clear();
        tracing::info!(target: "dakia_server::provider_io", operation = "sent_filing", "provider_io");
        let result = self.imap.reconcile_sent(message_id, raw).await;
        cache.clear();
        result
    }
}
