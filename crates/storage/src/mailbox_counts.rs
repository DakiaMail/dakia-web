use super::*;
use crate::classification::CLASSIFICATION_SQL;

impl Store {
    pub async fn mailbox_counts(&self, a: &AuthorizedAccount) -> Result<serde_json::Value> {
        let repository = self.account_repository(a).await?;
        let db = match &repository.backend {
            Backend::Sqlite(db) => db,
            Backend::Postgres(db) => {
                let starred: i64 = sqlx::query_scalar("SELECT count(*) FROM messages m WHERE account_id=$1 AND starred AND cardinality(folder_ids)>0 AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=$1 AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch)")
                    .bind(a.id()).fetch_one(db).await?;
                let attention: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM messages m CROSS JOIN (SELECT $3::TEXT AS filter) cf WHERE account_id=$1 AND m.upper_folders @> ARRAY['INBOX']::TEXT[] AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=m.account_id AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch) AND NOT EXISTS(SELECT 1 FROM operations o WHERE o.account_id=m.account_id AND o.target_id=m.id AND o.status='pending' AND o.change_kind IN ('archive','spam','trash')) AND NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.account_id=m.account_id AND s.thread_id=m.thread_id AND s.until>$2) {}", crate::classification::CLASSIFICATION_PG_SQL))
                    .bind(a.id()).bind(now()).bind(r#"{"attention":true}"#).fetch_one(db).await?;
                return Ok(serde_json::json!({ "starred": starred, "attention": attention }));
            }
        };
        let starred: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM messages m WHERE {}",
            cache::STARRED_PREDICATE
        ))
        .fetch_one(db)
        .await?;
        let attention: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM messages m CROSS JOIN (SELECT ? AS filter) cf WHERE 1=1 {} {} {CLASSIFICATION_SQL}", inbox::INBOX_PREDICATE, inbox::ATTENTION_PREDICATE
        )).bind(r#"{"attention":true}"#).bind(now()).fetch_one(db).await?;
        Ok(serde_json::json!({ "starred": starred, "attention": attention }))
    }
}
