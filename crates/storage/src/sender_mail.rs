use super::*;

impl Store {
    /// Local catalogue only. A cached count is not proof of provider coverage.
    /// Count and keyset page share a read snapshot; callers retain the cutoff.
    pub async fn cached_sender_mail(
        &self,
        a: &AuthorizedAccount,
        sender: &str,
        cutoff: &str,
        after: Option<&str>,
        trash: &[String],
    ) -> Result<(i64, Vec<Message>)> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let excluded = serde_json::to_string(trash)?;
        let filter = "json_extract(document,'$.sender.email')=? AND julianday(json_extract(document,'$.received_at'))<=julianday(?) AND NOT EXISTS (SELECT 1 FROM json_each(document,'$.folder_ids') f WHERE f.value IN (SELECT value FROM json_each(?)) OR lower(ltrim(f.value,'\\'))='trash')";
        let count = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM messages WHERE {filter}"))
            .bind(sender)
            .bind(cutoff)
            .bind(&excluded)
            .fetch_one(&mut *tx)
            .await?;
        let rows: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT document FROM messages WHERE {filter} AND id>? ORDER BY id LIMIT 101"
        ))
        .bind(sender)
        .bind(cutoff)
        .bind(&excluded)
        .bind(after.unwrap_or(""))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((
            count,
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect::<Result<_>>()?,
        ))
    }
}
