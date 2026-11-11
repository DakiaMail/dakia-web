use super::*;

impl Store {
    /// Snooze state is local, authoritative and scoped to an authorized account.
    pub async fn active_snoozes(&self, a: &AuthorizedAccount) -> Result<Vec<serde_json::Value>> {
        let db = self.db(a).await?;
        let rows = sqlx::query("SELECT s.thread_id,s.until,COUNT(m.id) AS message_count FROM snoozes s JOIN messages m ON json_extract(m.document,'$.thread_id')=s.thread_id WHERE s.until>?
            AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
            GROUP BY s.thread_id,s.until ORDER BY s.until,s.thread_id")
            .bind(now()).fetch_all(&db).await?;
        Ok(rows.into_iter().map(|row| serde_json::json!({
            "thread_id": row.get::<String,_>("thread_id"),
            "until": row.get::<String,_>("until"),
            "message_count": row.get::<i64,_>("message_count"),
        })).collect())
    }
}
