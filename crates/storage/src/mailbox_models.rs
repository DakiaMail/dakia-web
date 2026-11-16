use super::*;

impl MailboxTransaction<'_> {
    async fn defer_model_failure(
        &mut self,
        job: &ModelJob,
        reason: ModelFailureReason,
    ) -> Result<bool> {
        if job.account_id != self.account {
            return Err(Error::Invalid);
        }
        write!(self,"UPDATE model_jobs SET failure_reason=? WHERE id=? AND state='running'",[reason.code(),&job.id],"UPDATE model_jobs SET failure_reason=$1 WHERE account_id=$2 AND id=$3 AND state='running'",[reason.code(),&self.account,&job.id]);
        if !matches!(reason, ModelFailureReason::StorageUnavailable)
            && (!matches!(reason, ModelFailureReason::ContentUnavailable)
                || job.kind != JobKind::Classify)
        {
            return Ok(false);
        }
        let at = Utc::now().timestamp();
        let changed=write!(self,"UPDATE model_jobs SET state='pending',document=json_set(document,'$.status','pending'),retry_at=?+CASE WHEN retry_count=0 THEN 30 ELSE 120 END,retry_count=retry_count+1 WHERE id=? AND state='running' AND retry_count<2 AND EXISTS(SELECT 1 FROM messages WHERE id=json_extract(model_jobs.document,'$.message_id')) AND unixepoch(json_extract(document,'$.deadline'))>?+CASE WHEN retry_count=0 THEN 30 ELSE 120 END",[at,&job.id,at],"UPDATE model_jobs j SET state='pending',document=jsonb_set(document::jsonb,'{status}','\"pending\"')::TEXT,retry_at=$1+CASE WHEN retry_count=0 THEN 30 ELSE 120 END,retry_count=retry_count+1 WHERE account_id=$2 AND id=$3 AND state='running' AND retry_count<2 AND EXISTS(SELECT 1 FROM messages m WHERE m.account_id=j.account_id AND m.id=j.message_id ) AND floor(extract(epoch FROM deadline::TIMESTAMPTZ))::BIGINT>$1+CASE WHEN retry_count=0 THEN 30 ELSE 120 END",[at,&self.account,&job.id]);
        Ok(changed == 1)
    }
    async fn queue_model(
        &mut self,
        message: &str,
        kind: JobKind,
        model_revision: &str,
        source: Option<&str>,
    ) -> Result<ModelJob> {
        write!(
            self,
            "UPDATE messages SET revision=revision WHERE id=?",
            [message],
            "UPDATE messages SET revision=revision WHERE account_id=$1 AND id=$2",
            [&self.account, message]
        );
        let current = self.message(message).await?;
        let job = ModelJob {
            id: Uuid::new_v4().to_string(),
            account_id: self.account.clone(),
            message_id: message.into(),
            content_revision: current.revision,
            model_revision: model_revision.into(),
            kind,
            status: JobStatus::Pending,
            deadline: (Utc::now() + Duration::minutes(10)).to_rfc3339(),
        };
        let raw = serde_json::to_string(&job)?;
        write!(
            self,
            "INSERT INTO model_jobs(id,document,state) VALUES(?,?,'pending')",
            [&job.id, &raw],
            "INSERT INTO model_jobs(account_id,id,document,state) VALUES($1,$2,$3,'pending')",
            [&self.account, &job.id, &raw]
        );
        if let Some(source) = source {
            let key = format!("job-source:{}", job.id);
            write!(
                self,
                "INSERT INTO provider_documents(key,document) VALUES(?,?)",
                [&key, source],
                "INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3)",
                [&self.account, &key, source]
            );
        }
        Ok(job)
    }
    async fn claim_model(&mut self) -> Result<Option<ModelJob>> {
        let at = Utc::now().timestamp();
        write!(self,"UPDATE model_jobs SET state='failed',failure_reason='deadline',document=json_set(document,'$.status','failed') WHERE id IN (SELECT id FROM model_jobs WHERE state IN ('pending','running') AND state='running' AND unixepoch(json_extract(document,'$.deadline'))<=? LIMIT 200)",[at],"UPDATE model_jobs SET state='failed',failure_reason='deadline',document=jsonb_set(document::jsonb,'{status}','\"failed\"')::TEXT WHERE account_id=$1 AND id IN (SELECT id FROM model_jobs WHERE account_id=$1 AND state='running' AND floor(extract(epoch FROM deadline::TIMESTAMPTZ))::BIGINT<=$2 ORDER BY queue_id LIMIT 200)",[&self.account,at]);
        let raw:Option<String>=value!(self,"UPDATE model_jobs SET state='running' WHERE id=(SELECT id FROM model_jobs WHERE state='pending' AND retry_at<=? ORDER BY rowid LIMIT 1) AND state='pending' RETURNING document",[at],"UPDATE model_jobs SET state='running' WHERE account_id=$1 AND id=(SELECT id FROM model_jobs WHERE account_id=$1 AND state='pending' AND retry_at<=$2 ORDER BY queue_id LIMIT 1) AND state='pending' RETURNING document",[&self.account,at],fetch_optional);
        let Some(raw) = raw else {
            return Ok(None);
        };
        let mut job: ModelJob = serde_json::from_str(&raw)?;
        job.status = JobStatus::Running;
        let raw = serde_json::to_string(&job)?;
        write!(
            self,
            "UPDATE model_jobs SET document=? WHERE id=? AND state='running'",
            [&raw, &job.id],
            "UPDATE model_jobs SET document=$1 WHERE account_id=$2 AND id=$3 AND state='running'",
            [&raw, &self.account, &job.id]
        );
        Ok(Some(job))
    }
    async fn finish_model(
        &mut self,
        mut job: ModelJob,
        classification: Option<&Classification>,
        translation: Option<(&str, &MessageContent)>,
    ) -> Result<()> {
        if job.account_id != self.account
            || translation.is_some_and(|(_, content)| {
                content.message.account_id != self.account || content.message.id != job.message_id
            })
        {
            return Err(Error::Invalid);
        }
        let state: Option<String> = value!(
            self,
            "SELECT state FROM model_jobs WHERE id=?",
            [&job.id],
            "SELECT state FROM model_jobs WHERE account_id=$1 AND id=$2",
            [&self.account, &job.id],
            fetch_optional
        );
        if !matches!(
            state.ok_or(Error::NotFound)?.as_str(),
            "pending" | "running"
        ) {
            return Ok(());
        }
        write!(
            self,
            "UPDATE messages SET revision=revision WHERE id=?",
            [&job.message_id],
            "UPDATE messages SET revision=revision WHERE account_id=$1 AND id=$2",
            [&self.account, &job.message_id]
        );
        let mut message = self.message(&job.message_id).await?;
        if job.kind == JobKind::Classify && job.status == JobStatus::Cancelled {
            job.status = JobStatus::Cancelled;
            let raw = serde_json::to_string(&job)?;
            write!(self,"UPDATE model_jobs SET document=?,state='cancelled',failure_reason=NULL WHERE id=?",[&raw,&job.id],"UPDATE model_jobs SET document=$1,state='cancelled',failure_reason=NULL WHERE account_id=$2 AND id=$3",[&raw,&self.account,&job.id]);
            return Ok(());
        }
        let valid = message.revision == job.content_revision
            && DateTime::parse_from_rfc3339(&job.deadline)
                .map_err(|_| Error::Invalid)?
                .with_timezone(&Utc)
                > Utc::now();
        if !valid {
            job.status = JobStatus::Failed;
            let reason = if message.revision != job.content_revision {
                "revision_changed"
            } else {
                "deadline"
            };
            write!(
                self,
                "UPDATE model_jobs SET failure_reason=? WHERE id=?",
                [reason, &job.id],
                "UPDATE model_jobs SET failure_reason=$1 WHERE account_id=$2 AND id=$3",
                [reason, &self.account, &job.id]
            );
        } else if job.status == JobStatus::Succeeded {
            write!(
                self,
                "UPDATE model_jobs SET failure_reason=NULL WHERE id=?",
                [&job.id],
                "UPDATE model_jobs SET failure_reason=NULL WHERE account_id=$1 AND id=$2",
                [&self.account, &job.id]
            );
        }
        if valid && job.kind == JobKind::Classify {
            let failed = Classification {
                status: CategoryStatus::Failed,
                ..Default::default()
            };
            self.apply_classification(&mut message, Some(classification.unwrap_or(&failed)))
                .await?;
            write!(self,"UPDATE message_classifications SET requested=0 WHERE message_id=?",[&message.id],"UPDATE message_classifications SET requested=FALSE WHERE account_id=$1 AND message_id=$2",[&self.account,&message.id]);
            message.revision = message.revision.checked_add(1).ok_or(Error::Invalid)?;
            self.store_message(&message).await?;
        }
        if valid && job.status == JobStatus::Succeeded {
            if let Some((source, content)) = translation {
                let key = format!("translation:{}:{source}", job.message_id);
                let raw = serde_json::to_string(content)?;
                write!(self,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document",[&key,&raw],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&self.account,&key,&raw]);
            }
        }
        let state = wire(&job.status)?;
        let raw = serde_json::to_string(&job)?;
        write!(
            self,
            "UPDATE model_jobs SET document=?,state=? WHERE id=?",
            [&raw, &state, &job.id],
            "UPDATE model_jobs SET document=$1,state=$2 WHERE account_id=$3 AND id=$4",
            [&raw, &state, &self.account, &job.id]
        );
        self.event(message.revision, "message_changed", &message.id)
            .await?;
        Ok(())
    }
    async fn recover_models(&mut self) -> Result<()> {
        write!(self,"UPDATE model_jobs SET state='failed',failure_reason='interrupted',document=json_set(document,'$.status','failed') WHERE state='running'",[],"UPDATE model_jobs SET state='failed',failure_reason='interrupted',document=jsonb_set(document::jsonb,'{status}','\"failed\"')::TEXT WHERE account_id=$1 AND state='running'",[&self.account]);
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn model_failure_reason(&self, id: &str) -> Result<Option<String>> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(
                local,
                tx,
                "SELECT failure_reason FROM model_jobs WHERE id=?",
                [id],
                "SELECT failure_reason FROM model_jobs WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_one
            ))
        })
    }
    pub(crate) async fn model_job(&self, id: &str) -> Result<ModelJob> {
        transaction!(self, tx, local, false, {
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM model_jobs WHERE id=?",
                [id],
                "SELECT document FROM model_jobs WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
        })
    }
    pub(crate) async fn classification_candidate(&self) -> Result<Option<Message>> {
        transaction!(self, tx, local, false, {
            let raw:Option<String>=scalar!(local,tx,crate::models::CLASSIFICATION_CANDIDATE_QUERY,[],"SELECT m.document FROM messages m WHERE m.account_id=$1 AND m.document::jsonb#>>'{classification,status}'='pending' AND ('INBOX'=ANY(m.folder_ids) OR EXISTS(SELECT 1 FROM message_classifications c WHERE c.account_id=m.account_id AND c.message_id=m.id AND c.requested)) AND NOT EXISTS(SELECT 1 FROM model_jobs j WHERE j.account_id=m.account_id AND j.message_id=m.id AND j.state IN ('pending','running')) ORDER BY m.received_at DESC LIMIT 1",[&self.account],fetch_optional);
            raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
        })
    }
    pub(crate) async fn defer_model_failure(
        &self,
        job: &ModelJob,
        reason: ModelFailureReason,
    ) -> Result<bool> {
        scoped!(self, scope, true, {
            scope.defer_model_failure(job, reason).await
        })
    }
    pub(crate) async fn queue_model(
        &self,
        message: &str,
        kind: JobKind,
        revision: &str,
        source: Option<&str>,
    ) -> Result<ModelJob> {
        scoped!(self, scope, true, {
            scope.queue_model(message, kind, revision, source).await
        })
    }
    pub(crate) async fn claim_model(&self) -> Result<Option<ModelJob>> {
        scoped!(self, scope, true, { scope.claim_model().await })
    }
    pub(crate) async fn finish_model(
        &self,
        job: ModelJob,
        classification: Option<&Classification>,
        translation: Option<(&str, &MessageContent)>,
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope.finish_model(job, classification, translation).await
        })
    }
    pub(crate) async fn recover_models(&self) -> Result<()> {
        scoped!(self, scope, true, { scope.recover_models().await })
    }
}
