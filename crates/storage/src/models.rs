use super::*;
pub(crate) const CLASSIFICATION_CANDIDATE_QUERY: &str = "SELECT document FROM messages WHERE category_override IS NULL AND json_extract(document,'$.unread')=1 AND json_extract(document,'$.category_status')='pending' AND NOT EXISTS (SELECT 1 FROM model_jobs WHERE json_extract(model_jobs.document,'$.message_id')=messages.id AND state IN ('pending','running')) ORDER BY json_extract(document,'$.received_at') DESC LIMIT 1";
#[derive(Debug, Clone, Copy)]
pub enum ModelFailureReason {
    ContentUnavailable,
    ContentInvalid,
    RevisionChanged,
    WorkerFailed,
    Deadline,
}
impl ModelFailureReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::ContentUnavailable => "content_unavailable",
            Self::ContentInvalid => "content_invalid",
            Self::RevisionChanged => "revision_changed",
            Self::WorkerFailed => "worker_failed",
            Self::Deadline => "deadline",
        }
    }
}
impl Store {
    pub async fn model_failure_reason(
        &self,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<Option<String>> {
        let db = self.db(a).await?;
        Ok(
            sqlx::query_scalar("SELECT failure_reason FROM model_jobs WHERE id=?")
                .bind(id)
                .fetch_one(&db)
                .await?,
        )
    }
    /// Retain a content-free diagnosis. Only transient content acquisition can
    /// retry automatically, twice with durable backoff and the original deadline.
    pub async fn defer_model_failure(
        &self,
        a: &AuthorizedAccount,
        job: &ModelJob,
        reason: ModelFailureReason,
    ) -> Result<bool> {
        if job.account_id != a.id {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE model_jobs SET failure_reason=? WHERE id=? AND state='running'")
            .bind(reason.code())
            .bind(&job.id)
            .execute(&mut *tx)
            .await?;
        let can_retry = matches!(reason, ModelFailureReason::ContentUnavailable)
            && job.kind == JobKind::Classify;
        let changed = if can_retry {
            sqlx::query("UPDATE model_jobs SET state='pending',document=json_set(document,'$.status','pending'),retry_at=?+CASE WHEN retry_count=0 THEN 30 ELSE 120 END,retry_count=retry_count+1 WHERE id=? AND state='running' AND retry_count<2 AND EXISTS(SELECT 1 FROM messages WHERE id=json_extract(model_jobs.document,'$.message_id') AND json_extract(messages.document,'$.unread')=1) AND unixepoch(json_extract(document,'$.deadline'))>?+CASE WHEN retry_count=0 THEN 30 ELSE 120 END")
                .bind(Utc::now().timestamp()).bind(&job.id).bind(Utc::now().timestamp()).execute(&mut *tx).await?.rows_affected()==1
        } else {
            false
        };
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn model_job(&self, a: &AuthorizedAccount, id: &str) -> Result<ModelJob> {
        let db = self.db(a).await?;
        let raw: Option<String> = sqlx::query_scalar("SELECT document FROM model_jobs WHERE id=?")
            .bind(id)
            .fetch_optional(&db)
            .await?;
        Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
    }
    pub async fn queue_model(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        kind: JobKind,
        model_revision: &str,
        source: Option<&str>,
    ) -> Result<ModelJob> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE messages SET revision=revision WHERE id=?")
            .bind(message)
            .execute(&mut *tx)
            .await?;
        let revision: Option<u32> = sqlx::query_scalar("SELECT revision FROM messages WHERE id=?")
            .bind(message)
            .fetch_optional(&mut *tx)
            .await?;
        if kind == JobKind::Classify {
            let unread: Option<bool> = sqlx::query_scalar(
                "SELECT json_extract(document,'$.unread') FROM messages WHERE id=?",
            )
            .bind(message)
            .fetch_optional(&mut *tx)
            .await?;
            if !unread.ok_or(Error::NotFound)? {
                return Err(Error::Conflict);
            }
        }
        let job = ModelJob {
            id: Uuid::new_v4().to_string(),
            account_id: a.id.clone(),
            message_id: message.into(),
            content_revision: revision.ok_or(Error::NotFound)?,
            model_revision: model_revision.into(),
            kind,
            status: JobStatus::Pending,
            deadline: (Utc::now() + Duration::minutes(10)).to_rfc3339(),
        };
        sqlx::query("INSERT INTO model_jobs(id,document,state) VALUES(?,?,'pending')")
            .bind(&job.id)
            .bind(serde_json::to_string(&job)?)
            .execute(&mut *tx)
            .await?;
        if let Some(source) = source {
            sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?)")
                .bind(format!("job-source:{}", job.id))
                .bind(source)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(job)
    }
    pub async fn claim_model(&self, a: &AuthorizedAccount) -> Result<Option<ModelJob>> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        // Cancel even delayed retries, so old work cannot block a fresh unread job.
        // Cancellation leaves the message/category untouched and is not a failure.
        sqlx::query("UPDATE model_jobs SET state='cancelled',failure_reason=NULL,document=json_set(document,'$.status','cancelled') WHERE state='pending' AND json_extract(document,'$.kind')='classify' AND EXISTS(SELECT 1 FROM messages WHERE id=json_extract(model_jobs.document,'$.message_id') AND json_extract(messages.document,'$.unread')=0)")
            .execute(&mut *tx).await?;
        let raw:Option<String>=sqlx::query_scalar("UPDATE model_jobs SET state='running' WHERE id=(SELECT id FROM model_jobs WHERE state='pending' AND retry_at<=unixepoch('now') ORDER BY rowid LIMIT 1) AND state='pending' RETURNING document").fetch_optional(&mut *tx).await?;
        let Some(raw) = raw else {
            tx.commit().await?;
            return Ok(None);
        };
        let mut job: ModelJob = serde_json::from_str(&raw)?;
        job.status = JobStatus::Running;
        sqlx::query("UPDATE model_jobs SET document=? WHERE id=? AND state='running'")
            .bind(serde_json::to_string(&job)?)
            .bind(&job.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(job))
    }
    pub async fn classification_candidate(&self, a: &AuthorizedAccount) -> Result<Option<Message>> {
        let db = self.db(a).await?;
        let mut timing = timing::Timing::new("classification_candidate");
        let raw: Option<String> = sqlx::query_scalar(CLASSIFICATION_CANDIDATE_QUERY)
            .fetch_optional(&db)
            .await?;
        timing.stage("classification_decode");
        raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    /// Conditional commit prevents model completion overwriting manual changes or newer content.
    pub async fn finish_model(
        &self,
        a: &AuthorizedAccount,
        mut job: ModelJob,
        category: Option<&str>,
        translation: Option<(&str, &MessageContent)>,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        if job.account_id != a.id {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE messages SET revision=revision WHERE id=?")
            .bind(&job.message_id)
            .execute(&mut *tx)
            .await?;
        let raw: String = sqlx::query_scalar("SELECT document FROM messages WHERE id=?")
            .bind(&job.message_id)
            .fetch_one(&mut *tx)
            .await?;
        let mut message: Message = serde_json::from_str(&raw)?;
        if job.kind == JobKind::Classify && (!message.unread || job.status == JobStatus::Cancelled)
        {
            job.status = JobStatus::Cancelled;
            sqlx::query(
                "UPDATE model_jobs SET document=?,state='cancelled',failure_reason=NULL WHERE id=?",
            )
            .bind(serde_json::to_string(&job)?)
            .bind(&job.id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(());
        }
        let valid = message.revision == job.content_revision
            && DateTime::parse_from_rfc3339(&job.deadline)
                .map_err(|_| Error::Invalid)?
                .with_timezone(&Utc)
                > Utc::now();
        if !valid {
            job.status = JobStatus::Failed;
            sqlx::query("UPDATE model_jobs SET failure_reason=? WHERE id=?")
                .bind(if message.revision != job.content_revision {
                    "revision_changed"
                } else {
                    "deadline"
                })
                .bind(&job.id)
                .execute(&mut *tx)
                .await?;
        } else if job.status == JobStatus::Succeeded {
            sqlx::query("UPDATE model_jobs SET failure_reason=NULL WHERE id=?")
                .bind(&job.id)
                .execute(&mut *tx)
                .await?;
        }
        if valid && job.kind == JobKind::Classify {
            let manual: Option<String> =
                sqlx::query_scalar("SELECT category_override FROM messages WHERE id=?")
                    .bind(&job.message_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if manual.is_none() {
                message.category = category.map(str::to_owned);
                message.category_status = if category.is_some() {
                    CategoryStatus::Ready
                } else {
                    CategoryStatus::Failed
                };
                message.revision += 1;
                sqlx::query("UPDATE messages SET revision=?,document=? WHERE id=?")
                    .bind(message.revision)
                    .bind(serde_json::to_string(&message)?)
                    .bind(&message.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if valid && job.status == JobStatus::Succeeded {
            if let Some((source, content)) = translation {
                if content.message.account_id != a.id || content.message.id != job.message_id {
                    return Err(Error::Invalid);
                }
                sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document").bind(format!("translation:{}:{source}",job.message_id)).bind(serde_json::to_string(content)?).execute(&mut *tx).await?;
            }
        }
        let state = serde_json::to_value(&job.status)?
            .as_str()
            .ok_or(Error::Invalid)?
            .to_owned();
        sqlx::query("UPDATE model_jobs SET document=?,state=? WHERE id=?")
            .bind(serde_json::to_string(&job)?)
            .bind(state)
            .bind(&job.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'message_changed',?)")
            .bind(message.revision)
            .bind(&message.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn recover_models(&self, a: &AuthorizedAccount) -> Result<()> {
        let db = self.db(a).await?;
        sqlx::query("UPDATE model_jobs SET state='cancelled',failure_reason=NULL,document=json_set(document,'$.status','cancelled') WHERE state='running' AND json_extract(document,'$.kind')='classify' AND EXISTS(SELECT 1 FROM messages WHERE id=json_extract(model_jobs.document,'$.message_id') AND json_extract(messages.document,'$.unread')=0)")
            .execute(&db).await?;
        sqlx::query("UPDATE model_jobs SET state='failed',failure_reason='interrupted',document=json_set(document,'$.status','failed') WHERE state='running'").execute(&db).await?;
        Ok(())
    }
}
