use super::*;
const SQLITE_UNVERIFIED_MOVE: &str = "json_extract(document,'$.status') IN ('pending','failed') AND json_extract(document,'$.change.kind') IN ('archive','trash','spam','move') AND id NOT IN (SELECT operation_id FROM rule_executions WHERE state='cancelled' AND operation_id IS NOT NULL)";
const PG_UNVERIFIED_MOVE: &str = "status IN ('pending','failed') AND change_kind IN ('archive','trash','spam','move') AND id NOT IN (SELECT operation_id FROM rule_executions WHERE account_id=$1 AND state='cancelled' AND operation_id IS NOT NULL)";
fn key(id: &str) -> String {
    format!("sender-cleanup:{id}")
}
impl MailboxTransaction<'_> {
    async fn enqueue_sender_cleanup(&mut self, mut request: CleanupRequest) -> Result<CleanupJob> {
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
        let receipt = key(&request.id);
        write!(
            self,
            "UPDATE provider_documents SET document=document WHERE key=?",
            [&receipt],
            "UPDATE provider_documents SET document=document WHERE account_id=$1 AND key=$2",
            [&self.account, &receipt]
        );
        let existing: Option<String> = value!(
            self,
            "SELECT document FROM provider_documents WHERE key=?",
            [&receipt],
            "SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2",
            [&self.account, &receipt],
            fetch_optional
        );
        if let Some(raw) = existing {
            let old: CleanupJob = serde_json::from_str(&raw)?;
            if old.request != request {
                return Err(Error::Conflict);
            }
            return self.cleanup_status(old).await;
        }
        let sqlite = format!(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE json_extract(document,'$.target_id')=? AND {SQLITE_UNVERIFIED_MOVE})"
        );
        let pg = format!(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE account_id=$1 AND target_id=$2 AND {PG_UNVERIFIED_MOVE})"
        );
        let source = self.message(&request.source_id).await?;
        for (n, target) in request.targets.iter().enumerate() {
            let message = self.message(&target.id).await?;
            if message.account_id != self.account
                || message.sender.email.trim().to_lowercase()
                    != source.sender.email.trim().to_lowercase()
            {
                return Err(Error::Invalid);
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
                return Err(Error::Conflict);
            }
            let pending: bool = value!(
                self,
                &sqlite,
                [&target.id],
                &pg,
                [&self.account, &target.id],
                fetch_one
            );
            if pending {
                return Err(Error::CleanupMoveUnverified);
            }
            self.operation(Operation {
                id: format!("sender-cleanup:{}:{n}", request.id),
                account_id: self.account.clone(),
                target_id: target.id.clone(),
                expected_revision: message.revision,
                change: Change::Trash,
                status: OperationStatus::Pending,
                revision: 0,
            })
            .await?;
        }
        let job = CleanupJob {
            pending: request.targets.len(),
            request,
            account_id: self.account.clone(),
            sender: source.sender.email,
            created_at: now(),
            dismissed: false,
            applied: 0,
            failed: 0,
            failed_targets: vec![],
            projected_rows: vec![],
        };
        let raw = serde_json::to_string(&job)?;
        write!(
            self,
            "INSERT INTO provider_documents(key,document) VALUES(?,?)",
            [&receipt, &raw],
            "INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3)",
            [&self.account, &receipt, &raw]
        );
        Ok(job)
    }
    async fn cleanup_status(&mut self, mut job: CleanupJob) -> Result<CleanupJob> {
        job.applied = 0;
        job.failed = 0;
        job.pending = 0;
        job.failed_targets.clear();
        job.projected_rows.clear();
        let pattern = format!("sender-cleanup:{}:*", job.request.id);
        let lower = format!("sender-cleanup:{}:", job.request.id);
        let upper = format!("sender-cleanup:{};", job.request.id);
        let rows:Vec<(String,Option<String>)>=tuples!(self,"SELECT o.document,m.document FROM operations o LEFT JOIN messages m ON m.id=json_extract(o.document,'$.target_id') WHERE o.id GLOB ?",[&pattern],"SELECT o.document,m.document FROM operations o LEFT JOIN messages m ON m.account_id=o.account_id AND m.id=o.target_id WHERE o.account_id=$1 AND o.id COLLATE \"C\">=$2 AND o.id COLLATE \"C\"<$3",[&self.account,&lower,&upper],fetch_all);
        if rows.len() != job.request.targets.len() {
            return Err(Error::Invalid);
        }
        for (raw, message) in rows {
            let operation: Operation = serde_json::from_str(&raw)?;
            match operation.status {
                OperationStatus::Applied => job.applied += 1,
                OperationStatus::Failed => {
                    job.failed += 1;
                    job.failed_targets.push(operation.target_id);
                }
                OperationStatus::Pending => {
                    job.pending += 1;
                    if let Some(raw) = message {
                        let mut message: Message = serde_json::from_str(&raw)?;
                        message.folder_ids = vec!["Trash".into()];
                        job.projected_rows.push(message);
                    }
                }
            }
        }
        Ok(job)
    }
    async fn sender_cleanup_jobs(&mut self) -> Result<Vec<CleanupJob>> {
        // Guard decoding with the receipt prefix: other provider state is opaque.
        let rows:Vec<String>=value!(self,"SELECT document FROM provider_documents WHERE key GLOB 'sender-cleanup:*' AND json_extract(document,'$.dismissed')=0 ORDER BY key",[],"SELECT document FROM provider_documents WHERE account_id=$1 AND key COLLATE \"C\">='sender-cleanup:' AND key COLLATE \"C\"<'sender-cleanup;' AND CASE WHEN left(key,15)='sender-cleanup:' THEN (document::jsonb->>'dismissed')::BOOLEAN ELSE TRUE END=FALSE ORDER BY key COLLATE \"C\"",[&self.account],fetch_all);
        let mut jobs = Vec::with_capacity(rows.len());
        for raw in rows {
            jobs.push(self.cleanup_status(serde_json::from_str(&raw)?).await?);
        }
        Ok(jobs)
    }
    async fn dismiss_sender_cleanup(&mut self, id: &str) -> Result<()> {
        if Uuid::parse_str(id).is_err() {
            return Err(Error::Invalid);
        }
        let key = key(id);
        write!(
            self,
            "UPDATE provider_documents SET document=document WHERE key=?",
            [&key],
            "UPDATE provider_documents SET document=document WHERE account_id=$1 AND key=$2",
            [&self.account, &key]
        );
        let raw: Option<String> = value!(
            self,
            "SELECT document FROM provider_documents WHERE key=?",
            [&key],
            "SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2",
            [&self.account, &key],
            fetch_optional
        );
        let mut job = self
            .cleanup_status(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
            .await?;
        if job.pending != 0 {
            return Err(Error::Conflict);
        }
        job.dismissed = true;
        let raw = serde_json::to_string(&job)?;
        write!(
            self,
            "UPDATE provider_documents SET document=? WHERE key=?",
            [&raw, &key],
            "UPDATE provider_documents SET document=$1 WHERE account_id=$2 AND key=$3",
            [&raw, &self.account, &key]
        );
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn enqueue_sender_cleanup(
        &self,
        request: CleanupRequest,
    ) -> Result<CleanupJob> {
        scoped!(self, scope, true, {
            scope.enqueue_sender_cleanup(request).await
        })
    }
    pub(crate) async fn sender_cleanup_jobs(&self) -> Result<Vec<CleanupJob>> {
        scoped!(self, scope, false, { scope.sender_cleanup_jobs().await })
    }
    pub(crate) async fn dismiss_sender_cleanup(&self, id: &str) -> Result<()> {
        scoped!(self, scope, true, {
            scope.dismiss_sender_cleanup(id).await
        })
    }
    pub(crate) async fn cached_sender_mail(
        &self,
        sender: &str,
        cutoff: &str,
        after: Option<&str>,
        trash: &[String],
    ) -> Result<(i64, Vec<Message>)> {
        let sender = sender.trim().to_lowercase();
        let trash: Vec<_> = trash.iter().map(|folder| folder.to_lowercase()).collect();
        let excluded = serde_json::to_string(&trash)?;
        let mut sqlite = r"lower(trim(json_extract(m.document,'$.sender.email')))=? AND julianday(json_extract(m.document,'$.received_at'))<=julianday(?) AND NOT EXISTS (SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE lower(f.value) IN (SELECT value FROM json_each(?)) OR lower(ltrim(f.value,'\'))='trash')".to_owned();
        let mut pg = r"m.account_id=$1 AND lower(trim(m.document::jsonb#>>'{sender,email}'))=$2 AND mail_julianday(m.received_at)<=mail_julianday($3) AND NOT EXISTS(SELECT 1 FROM unnest(m.folder_ids) f WHERE lower(f)=ANY($4::TEXT[]) OR lower(ltrim(f,'\'))='trash')".to_owned();
        sqlite.push_str(&format!(" AND json_array_length(m.document,'$.folder_ids')>0 AND NOT EXISTS (SELECT 1 FROM operations WHERE json_extract(document,'$.target_id')=m.id AND {SQLITE_UNVERIFIED_MOVE})"));
        pg.push_str(&format!(" AND cardinality(m.folder_ids)>0 AND NOT EXISTS (SELECT 1 FROM operations WHERE account_id=$1 AND target_id=m.id AND {PG_UNVERIFIED_MOVE})"));
        let rows: Vec<String>;
        let count: i64;
        match &self.backend {
            Backend::Sqlite(db) => {
                let mut tx = db.begin().await?;
                count =
                    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM messages m WHERE {sqlite}"))
                        .bind(&sender)
                        .bind(cutoff)
                        .bind(&excluded)
                        .fetch_one(&mut *tx)
                        .await?;
                rows = sqlx::query_scalar(&format!(
                    "SELECT document FROM messages m WHERE {sqlite} AND id>? ORDER BY id LIMIT 101"
                ))
                .bind(&sender)
                .bind(cutoff)
                .bind(&excluded)
                .bind(after.unwrap_or(""))
                .fetch_all(&mut *tx)
                .await?;
                tx.commit().await?;
            }
            Backend::Postgres(db) => {
                let mut tx = self.begin_postgres(db, false).await?;
                // One statement gives count and page the same snapshot without
                // strengthening isolation around the ownership row lock.
                (count,rows)=sqlx::query_as(&format!("SELECT (SELECT COUNT(*) FROM messages m WHERE {pg}),ARRAY(SELECT document FROM messages m WHERE {pg} AND id COLLATE \"C\">$5 ORDER BY id COLLATE \"C\" LIMIT 101)")).bind(&self.account).bind(&sender).bind(cutoff).bind(&trash).bind(after.unwrap_or("")).fetch_one(&mut *tx).await?;
                tx.commit().await?;
            }
        }
        Ok((
            count,
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect::<Result<_>>()?,
        ))
    }
}
