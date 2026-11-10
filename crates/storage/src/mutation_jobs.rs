use super::*;
impl Store {
    /// Claim exactly once. A process lost after claim must reconcile, never replay a move.
    pub async fn claim_mutation(&self, a: &AuthorizedAccount) -> Result<Option<Operation>> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        self.cancel_disabled_rules_tx(&mut tx, a).await?;
        let target: Option<String> = sqlx::query_scalar("UPDATE outbox SET state='running' WHERE id=(SELECT q.id FROM outbox q JOIN operations o ON o.id=q.target_id WHERE q.kind='mutation' AND q.state='pending' AND (q.lease_until IS NULL OR q.lease_until<=?) AND NOT EXISTS(SELECT 1 FROM outbox prior JOIN operations p ON p.id=prior.target_id WHERE prior.kind='mutation' AND prior.state IN ('pending','running') AND prior.id<q.id AND json_extract(p.document,'$.target_id')=json_extract(o.document,'$.target_id')) ORDER BY q.id LIMIT 1) AND state='pending' RETURNING target_id")
            .bind(now()).fetch_optional(&mut *tx).await?;
        let Some(target) = target else {
            return Ok(None);
        };
        let raw: String = sqlx::query_scalar("SELECT document FROM operations WHERE id=?")
            .bind(target)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(serde_json::from_str(&raw)?))
    }
    /// Only call after a provider proves no write was attempted. Persist retry
    /// backoff and its budget atomically, preserving per-message operation order.
    pub async fn retry_mutation(&self, a: &AuthorizedAccount, id: &str) -> Result<bool> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let key = format!("mutation-retry:{id}");
        let attempts: Option<String> =
            sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=?")
                .bind(&key)
                .fetch_optional(&mut *tx)
                .await?;
        let attempts = attempts
            .map(|s| s.parse::<u32>().map_err(|_| Error::Invalid))
            .transpose()?
            .unwrap_or(0);
        if attempts >= 3 {
            return Ok(false);
        }
        let due = (Utc::now() + Duration::seconds(5 * (1 << attempts)))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let changed = sqlx::query("UPDATE outbox SET state='pending',lease_until=? WHERE kind='mutation' AND target_id=? AND state='running'").bind(due).bind(id).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document").bind(key).bind((attempts+1).to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
    pub async fn finish_mutation(
        &self,
        a: &AuthorizedAccount,
        mut op: Operation,
        folders: Option<Vec<String>>,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE operations SET document=document WHERE id=?")
            .bind(&op.id)
            .execute(&mut *tx)
            .await?;
        let raw: String = sqlx::query_scalar("SELECT document FROM messages WHERE id=?")
            .bind(&op.target_id)
            .fetch_one(&mut *tx)
            .await?;
        let mut message: Message = serde_json::from_str(&raw)?;
        if let Some(folders) = folders {
            message.folder_ids = folders;
        }
        // Clear the thread's snooze only after the provider confirms Inbox restoration.
        // A rejected or uncertain move must retain the original snooze and deadline.
        if op.status == OperationStatus::Applied
            && matches!(&op.change, Change::Move { folder_id } if folder_id.eq_ignore_ascii_case("INBOX"))
        {
            sqlx::query("DELETE FROM snoozes WHERE thread_id=?")
                .bind(&message.thread_id).execute(&mut *tx).await?;
        }
        message.revision = message.revision.checked_add(1).ok_or(Error::Invalid)?;
        op.revision = message.revision;
        sqlx::query("UPDATE messages SET revision=?,document=? WHERE id=?")
            .bind(message.revision)
            .bind(serde_json::to_string(&message)?)
            .bind(&message.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE operations SET document=? WHERE id=?")
            .bind(serde_json::to_string(&op)?)
            .bind(&op.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE outbox SET state='done' WHERE kind='mutation' AND target_id=?")
            .bind(&op.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM provider_documents WHERE key=?")
            .bind(format!("mutation-retry:{}", op.id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'operation_changed',?)")
            .bind(op.revision)
            .bind(&op.target_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn recover_mutations(&self, a: &AuthorizedAccount) -> Result<()> {
        let db = self.db(a).await?;
        let rows: Vec<String> = sqlx::query_scalar("SELECT operations.document FROM operations JOIN outbox ON outbox.target_id=operations.id WHERE outbox.kind='mutation' AND outbox.state='running'").fetch_all(&db).await?;
        for raw in rows {
            let mut op: Operation = serde_json::from_str(&raw)?;
            op.status = OperationStatus::Failed;
            self.finish_mutation(a, op, None).await?;
        }
        Ok(())
    }
    pub async fn link_identity(
        &self,
        a: &AuthorizedAccount,
        provider_id: &str,
        message_id: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        sqlx::query("INSERT INTO message_identities(provider_id,message_id) VALUES(?,?) ON CONFLICT(provider_id) DO UPDATE SET message_id=excluded.message_id").bind(provider_id).bind(message_id).execute(&db).await?;
        Ok(())
    }
}
