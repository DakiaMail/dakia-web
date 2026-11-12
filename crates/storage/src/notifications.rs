use super::*;

// Resolve current classification and attention again at the send boundary.
const PUSH_ELIGIBLE: &str = "SELECT id FROM messages m WHERE m.id=push_jobs.message_id AND (COALESCE(json_extract(m.document,'$.classification.kind'),'personal')='personal' OR (json_extract(m.document,'$.classification.attention_override')=1 OR (json_extract(m.document,'$.classification.attention_override') IS NULL AND EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.attention') reason WHERE json_extract(reason.value,'$.expires_at') IS NULL OR json_extract(reason.value,'$.expires_at')>strftime('%Y-%m-%dT%H:%M:%fZ','now'))))) AND (json_extract(m.document,'$.classification.status') IN ('ready','failed') OR json_extract(m.document,'$.classification.source') IN ('message','sender') OR json_extract(m.document,'$.classification.attention_override')=1)";
const PUSH_SUPPRESSED: &str = "SELECT id FROM messages m WHERE m.id=push_jobs.message_id AND json_extract(m.document,'$.classification.kind')='automated' AND json_extract(m.document,'$.classification.status') IN ('ready','failed') AND NOT COALESCE((json_extract(m.document,'$.classification.attention_override')=1 OR (json_extract(m.document,'$.classification.attention_override') IS NULL AND EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.attention') reason WHERE json_extract(reason.value,'$.expires_at') IS NULL OR json_extract(reason.value,'$.expires_at')>strftime('%Y-%m-%dT%H:%M:%fZ','now')))),0)";
impl Store {
    pub async fn subscribe(&self, user: &str, id: &str, document: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        sqlx::query("INSERT INTO push_endpoints(id,user_id,document) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET user_id=excluded.user_id,document=excluded.document").bind(id).bind(user).bind(document).execute(&self.control).await?;
        Ok(())
    }
    pub async fn subscriptions(&self, a: &AuthorizedAccount) -> Result<Vec<serde_json::Value>> {
        self.authorize(&a.user_id, &a.id).await?;
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT document FROM push_endpoints WHERE user_id=?")
                .bind(&a.user_id)
                .fetch_all(&self.control)
                .await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub async fn expire_subscription(&self, a: &AuthorizedAccount, id: &str) -> Result<()> {
        self.authorize(&a.user_id, &a.id).await?;
        sqlx::query("DELETE FROM push_endpoints WHERE id=? AND user_id=?")
            .bind(id)
            .bind(&a.user_id)
            .execute(&self.control)
            .await?;
        Ok(())
    }
    pub async fn next_push(&self, a: &AuthorizedAccount) -> Result<Option<serde_json::Value>> {
        let _rules_guard = self.rules_gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        Self::suppress_push_tx(&mut tx).await?;
        self.suppress_rule_push_tx(&mut tx, a).await?;
        // Pending classification does not consume delivery attempts or block
        // other eligible mail. A completed classification failure falls back to
        // ordinary mail notifications; it is not evidence of a newsletter.
        let raw:Option<String>=sqlx::query_scalar(&format!("UPDATE push_jobs SET state='running',document=CASE WHEN attempts=0 THEN json_set(document,'$.deadline',?) ELSE document END,attempts=attempts+1 WHERE rowid=(SELECT rowid FROM push_jobs WHERE state='pending' AND next_attempt<=? AND attempts<5 AND EXISTS ({PUSH_ELIGIBLE}) ORDER BY rowid LIMIT 1) RETURNING document"))
            .bind((Utc::now()+Duration::minutes(5)).to_rfc3339()).bind(Utc::now().timestamp()).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }

    async fn suppress_push_tx(tx: &mut sqlx::SqliteConnection) -> Result<()> {
        sqlx::query(&format!("UPDATE push_jobs SET state='skipped' WHERE state IN ('pending','running') AND (EXISTS ({PUSH_SUPPRESSED}) OR NOT EXISTS(SELECT 1 FROM messages WHERE id=push_jobs.message_id))"))
            .execute(&mut *tx).await?;
        Ok(())
    }

    /// Final send decision after subscription resolution, not a cached queue
    /// admission decision. Reclassification and legacy/recovered jobs use the
    /// same policy. A waiting job retains its attempts for actual transport.
    pub async fn push_send_allowed(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        subscription: &str,
    ) -> Result<bool> {
        let _rules_guard = self.rules_gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        Self::suppress_push_tx(&mut tx).await?;
        self.suppress_rule_push_tx(&mut tx, a).await?;
        let allowed: bool = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT 1 FROM push_jobs WHERE message_id=? AND subscription_id=? AND state='running' AND EXISTS ({PUSH_ELIGIBLE}))"))
            .bind(message).bind(subscription).fetch_one(&mut *tx).await?;
        if !allowed {
            sqlx::query("UPDATE push_jobs SET state='pending',attempts=MAX(0,attempts-1) WHERE message_id=? AND subscription_id=? AND state='running'")
                .bind(message).bind(subscription).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(allowed)
    }
    pub async fn finish_push(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        subscription: &str,
        retry: Option<u32>,
    ) -> Result<()> {
        let db = self.db(a).await?;
        sqlx::query(
            "UPDATE push_jobs SET state=?,next_attempt=? WHERE message_id=? AND subscription_id=? AND state='running'",
        )
        .bind(if retry.is_some() { "pending" } else { "done" })
        .bind(Utc::now().timestamp() + i64::from(retry.unwrap_or(0).min(3600)))
        .bind(message)
        .bind(subscription)
        .execute(&db)
        .await?;
        Ok(())
    }
    pub async fn recover_push(&self, a: &AuthorizedAccount) -> Result<()> {
        let _rules_guard = self.rules_gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        Self::suppress_push_tx(&mut tx).await?;
        self.suppress_rule_push_tx(&mut tx, a).await?;
        // Stable notification tag deduplicates a retry after an uncertain HTTP outcome.
        sqlx::query("UPDATE push_jobs SET state='pending' WHERE state='running'")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn is_snoozed(&self, a: &AuthorizedAccount, thread: &str) -> Result<bool> {
        let db = self.db(a).await?;
        Ok(
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM snoozes WHERE thread_id=? AND until>?)",
            )
            .bind(thread)
            .bind(now())
            .fetch_one(&db)
            .await?,
        )
    }
    pub async fn wake_snoozes(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let rows: Vec<String> =
            sqlx::query_scalar("DELETE FROM snoozes WHERE until<=? RETURNING thread_id")
                .bind(now())
                .fetch_all(&mut *tx)
                .await?;
        for thread in rows {
            sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(0,'snooze_woke',?)")
                .bind(thread)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
