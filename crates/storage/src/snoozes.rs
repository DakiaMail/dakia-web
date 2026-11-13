use super::*;

impl Store {
    /// Snoozes are authoritative local state; opening the view needs no provider scan.
    pub async fn snoozed_rows_filtered(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
        filter: &ClassificationFilter,
    ) -> Result<Vec<Message>> {
        let db = self.db(a).await?;
        let rows: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT m.document FROM snoozes s CROSS JOIN messages m CROSS JOIN (SELECT ? AS filter) cf
             WHERE s.until>? AND CAST(json_extract(m.document,'$.thread_id') AS TEXT)=s.thread_id
             AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             {CLASSIFICATION_SQL}
             AND (? IS NULL OR json_extract(m.document,'$.category') IS NULL OR json_extract(m.document,'$.category')=?)
             AND (? IS NULL OR (json_extract(m.document,'$.received_at'),m.id)<(?,?))
             ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 51"
        )).bind(serde_json::to_string(filter)?).bind(now())
            .bind(category).bind(category)
            .bind(before.map(|b| b.0)).bind(before.map(|b| b.0)).bind(before.map(|b| b.1))
            .fetch_all(&db).await?;
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }

    /// Snooze state is local, authoritative and scoped to an authorized account.
    pub async fn active_snoozes(&self, a: &AuthorizedAccount) -> Result<Vec<serde_json::Value>> {
        let db = self.db(a).await?;
        let rows = sqlx::query("SELECT s.thread_id,s.until,COUNT(m.id) AS message_count FROM snoozes s CROSS JOIN messages m ON CAST(json_extract(m.document,'$.thread_id') AS TEXT)=s.thread_id WHERE s.until>?
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
