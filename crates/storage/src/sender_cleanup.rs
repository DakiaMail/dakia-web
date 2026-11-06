//! A frozen sender-cleanup batch groups the existing durable mutation outbox.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupTarget {
    pub id: String,
    pub expected_revision: u32,
    /// Flags and classification may change while the owner reviews the batch.
    /// A location snapshot permits that drift, never a different message or move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_location: Option<CleanupLocation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupLocation {
    pub provider_id: String,
    pub folder_ids: Vec<String>,
    pub sender_email: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRequest {
    pub id: String,
    pub source_id: String,
    pub targets: Vec<CleanupTarget>,
    pub search_complete: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CleanupJob {
    pub request: CleanupRequest,
    pub account_id: String,
    pub sender: String,
    pub created_at: String,
    pub dismissed: bool,
    pub applied: usize,
    pub failed: usize,
    pub pending: usize,
    #[serde(default)]
    pub failed_targets: Vec<String>,
    /// Response-only optimistic metadata. Never a provider outcome or stored
    /// membership; allows other/reloaded clients to show pending mail in Trash.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projected_rows: Vec<Message>,
}
fn key(id: &str) -> String {
    format!("sender-cleanup:{id}")
}
fn operation_id(job: &str, n: usize) -> String {
    format!("sender-cleanup:{job}:{n}")
}
impl Store {
    pub async fn enqueue_sender_cleanup(
        &self,
        a: &AuthorizedAccount,
        mut request: CleanupRequest,
    ) -> Result<CleanupJob> {
        if Uuid::parse_str(&request.id).is_err()
            || request.targets.is_empty()
            || request.targets.len() > 10000
        {
            return Err(Error::Invalid);
        }
        request.targets.sort_by(|a, b| a.id.cmp(&b.id));
        if request.targets.windows(2).any(|v| v[0].id == v[1].id) {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        // Acquire the SQLite write lock before checking idempotency and overlap.
        sqlx::query("UPDATE provider_documents SET document=document WHERE key=?")
            .bind(key(&request.id))
            .execute(&mut *tx)
            .await?;
        let existing: Vec<String> = sqlx::query_scalar(
            "SELECT document FROM provider_documents WHERE key GLOB 'sender-cleanup:*'",
        )
        .fetch_all(&mut *tx)
        .await?;
        for raw in existing {
            let old: CleanupJob = serde_json::from_str(&raw)?;
            if old.request.id == request.id {
                if old.request != request {
                    return Err(Error::Conflict);
                }
                return Self::cleanup_status_tx(&mut tx, old).await;
            }
            // No fresh operation IDs for previously accepted targets, including
            // uncertain outcomes. The owner must reconcile before another action.
            if old.request.targets.iter().any(|t| {
                request
                    .targets
                    .binary_search_by(|v| v.id.cmp(&t.id))
                    .is_ok()
            }) {
                return Err(Error::Conflict);
            }
        }
        let source: String = sqlx::query_scalar("SELECT document FROM messages WHERE id=?")
            .bind(&request.source_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        let source: Message = serde_json::from_str(&source)?;
        for (n, target) in request.targets.iter().enumerate() {
            let raw: String = sqlx::query_scalar("SELECT document FROM messages WHERE id=?")
                .bind(&target.id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::NotFound)?;
            let message: Message = serde_json::from_str(&raw)?;
            if message.account_id != a.id || message.sender.email != source.sender.email {
                return Err(Error::Invalid);
            }
            let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM operations WHERE json_extract(document,'$.target_id')=? AND json_extract(document,'$.status') IN ('pending','failed') AND json_extract(document,'$.change.kind') IN ('archive','trash','spam','move')").bind(&target.id).fetch_one(&mut *tx).await?;
            if pending != 0 {
                return Err(Error::CleanupMoveUnverified);
            }
            if let Some(location) = &target.expected_location {
                let mut expected = location.folder_ids.clone();
                let mut current = message.folder_ids.clone();
                expected.sort();
                current.sort();
                if target.expected_revision > message.revision
                    || target.expected_revision == 0
                    || location.provider_id != message.provider_id
                    || location.sender_email != message.sender.email
                    || expected != current
                    || current.is_empty()
                {
                    return Err(Error::Conflict);
                }
            } else if target.expected_revision != message.revision {
                // Old saved requests remain strict and are never rewritten/retried.
                return Err(Error::Conflict);
            }
            Self::operation_tx(
                &mut tx,
                a,
                Operation {
                    id: operation_id(&request.id, n),
                    account_id: a.id.clone(),
                    target_id: target.id.clone(),
                    expected_revision: message.revision,
                    change: Change::Trash,
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await?;
        }
        let job = CleanupJob {
            pending: request.targets.len(),
            request,
            account_id: a.id.clone(),
            sender: source.sender.email,
            created_at: now(),
            dismissed: false,
            applied: 0,
            failed: 0,
            failed_targets: Vec::new(),
            projected_rows: Vec::new(),
        };
        sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?)")
            .bind(key(&job.request.id))
            .bind(serde_json::to_string(&job)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(job)
    }
    async fn cleanup_status_tx(
        tx: &mut sqlx::SqliteConnection,
        mut job: CleanupJob,
    ) -> Result<CleanupJob> {
        job.applied = 0;
        job.failed = 0;
        job.pending = 0;
        job.failed_targets.clear();
        job.projected_rows.clear();
        let rows =
            sqlx::query("SELECT o.document, m.document AS message FROM operations o LEFT JOIN messages m ON m.id=json_extract(o.document,'$.target_id') WHERE o.id GLOB ?")
                .bind(format!("sender-cleanup:{}:*", job.request.id))
                .fetch_all(&mut *tx)
                .await?;
        if rows.len() != job.request.targets.len() {
            return Err(Error::Invalid);
        }
        for row in rows {
            let op: Operation = serde_json::from_str(row.get("document"))?;
            match op.status {
                OperationStatus::Applied => job.applied += 1,
                OperationStatus::Failed => {
                    job.failed += 1;
                    job.failed_targets.push(op.target_id);
                }
                OperationStatus::Pending => {
                    job.pending += 1;
                    if let Some(raw) = row.get::<Option<String>, _>("message") {
                        let mut message: Message = serde_json::from_str(&raw)?;
                        message.folder_ids = vec!["Trash".into()];
                        job.projected_rows.push(message);
                    }
                }
            }
        }
        Ok(job)
    }
    pub async fn sender_cleanup_jobs(&self, a: &AuthorizedAccount) -> Result<Vec<CleanupJob>> {
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let rows: Vec<String> = sqlx::query_scalar("SELECT document FROM provider_documents WHERE key GLOB 'sender-cleanup:*' AND json_extract(document,'$.dismissed')=0 ORDER BY key").fetch_all(&mut *tx).await?;
        let mut jobs = Vec::new();
        for raw in rows {
            jobs.push(Self::cleanup_status_tx(&mut tx, serde_json::from_str(&raw)?).await?);
        }
        Ok(jobs)
    }
    pub async fn dismiss_sender_cleanup(&self, a: &AuthorizedAccount, id: &str) -> Result<()> {
        if Uuid::parse_str(id).is_err() {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE provider_documents SET document=document WHERE key=?")
            .bind(key(id))
            .execute(&mut *tx)
            .await?;
        let raw: String = sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=?")
            .bind(key(id))
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        let mut job = Self::cleanup_status_tx(&mut tx, serde_json::from_str(&raw)?).await?;
        if job.pending != 0 {
            return Err(Error::Conflict);
        }
        job.dismissed = true;
        sqlx::query("UPDATE provider_documents SET document=? WHERE key=?")
            .bind(serde_json::to_string(&job)?)
            .bind(key(id))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
