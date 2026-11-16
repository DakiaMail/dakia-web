use super::*;

impl Store {
    pub async fn mailbox_counts(&self, a: &AuthorizedAccount) -> Result<serde_json::Value> {
        let db = self.db(a).await?;
        let starred: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM messages m WHERE {}",
            cache::STARRED_PREDICATE
        ))
        .fetch_one(&db)
        .await?;
        let attention: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM messages m CROSS JOIN (SELECT ? AS filter) cf WHERE 1=1 {} {} {CLASSIFICATION_SQL}", inbox::INBOX_PREDICATE, inbox::ATTENTION_PREDICATE
        )).bind(r#"{"attention":true}"#).bind(now()).fetch_one(&db).await?;
        Ok(serde_json::json!({ "starred": starred, "attention": attention }))
    }
}
