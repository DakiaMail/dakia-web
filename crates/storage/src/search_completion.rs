//! Reverse outbox for stage completion. A target receipt is acknowledged only
//! after the independently selected application store persisted its decision.
use super::*;
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct CompletionReceipt {
    pub message_id: String,
    pub version: i64,
    pub source: String,
    pub model_revision: String,
    pub content_version: i64,
    pub lexical_revision: i64,
}
type ReceiptRow = (String, i64, String, String, i64, i64);
impl SearchDatabase {
    pub async fn completion_receipts(&self, account: &str) -> Result<Vec<CompletionReceipt>> {
        let rows:Vec<ReceiptRow>=match self {
            Self::Sqlite(db)=>sqlx::query_as("SELECT message_id,version,source,model_revision,content_version,lexical_revision FROM embedding_receipts WHERE account_id=? ORDER BY version,message_id LIMIT 200").bind(account).fetch_all(db).await?,
            Self::Postgres(db)=>sqlx::query_as("SELECT message_id,version,source,model_revision,content_version,lexical_revision FROM embedding_receipts WHERE account_id=$1 ORDER BY version,message_id COLLATE \"C\" LIMIT 200").bind(account).fetch_all(db).await?,
        };
        Ok(rows
            .into_iter()
            .map(
                |(
                    message_id,
                    version,
                    source,
                    model_revision,
                    content_version,
                    lexical_revision,
                )| {
                    CompletionReceipt {
                        message_id,
                        version,
                        source,
                        model_revision,
                        content_version,
                        lexical_revision,
                    }
                },
            )
            .collect())
    }
    pub async fn acknowledge_completion(
        &self,
        account: &str,
        receipts: &[CompletionReceipt],
    ) -> Result<()> {
        if receipts.len() > 200 {
            return Err(Error::Invalid);
        }
        if receipts.is_empty() {
            return Ok(());
        }
        let document = serde_json::to_string(receipts)?;
        macro_rules! acknowledge {
            ($db:expr,$sql:expr,$pg:expr) => {{
                let mut tx = $db.begin().await?;
                if $pg {
                    sqlx::query(
                        "SELECT pg_advisory_xact_lock(hashtextextended($1,19217639535198210))",
                    )
                    .bind(account)
                    .execute(&mut *tx)
                    .await?;
                }
                sqlx::query($sql)
                    .bind(account)
                    .bind(&document)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }};
        }
        match self {
            Self::Sqlite(db)=>acknowledge!(db,"DELETE FROM embedding_receipts WHERE account_id=?1 AND message_id IN (SELECT json_extract(value,'$.message_id') FROM json_each(?2)) AND EXISTS(SELECT 1 FROM json_each(?2) r WHERE json_extract(r.value,'$.message_id')=embedding_receipts.message_id AND json_extract(r.value,'$.version')=embedding_receipts.version AND json_extract(r.value,'$.model_revision')=embedding_receipts.model_revision AND json_extract(r.value,'$.content_version')=embedding_receipts.content_version AND json_extract(r.value,'$.lexical_revision')=embedding_receipts.lexical_revision)",false),
            Self::Postgres(db)=>acknowledge!(db,"DELETE FROM embedding_receipts e USING jsonb_to_recordset($2::jsonb) r(message_id TEXT,version BIGINT,model_revision TEXT,content_version BIGINT,lexical_revision BIGINT) WHERE e.account_id=$1 AND e.message_id=r.message_id AND e.version=r.version AND e.model_revision=r.model_revision AND e.content_version=r.content_version AND e.lexical_revision=r.lexical_revision",true),
        }
        Ok(())
    }
}
impl SearchConnection {
    pub async fn completion_receipts(&self, account: &str) -> Result<Vec<CompletionReceipt>> {
        self.connection().await?.completion_receipts(account).await
    }
    pub async fn acknowledge_completion(
        &self,
        account: &str,
        receipts: &[CompletionReceipt],
    ) -> Result<()> {
        self.connection()
            .await?
            .acknowledge_completion(account, receipts)
            .await
    }
}
impl crate::Store {
    /// Configure before sharing the server Store, so a cold/missing model never
    /// lets lexical-only history claim processing completion during startup.
    pub fn require_search_embeddings(&mut self, revision: &str) -> Result<()> {
        if revision.is_empty() || revision.len() > 128 || revision.contains('\0') {
            return Err(Error::Invalid);
        }
        self.expected_search_model = Some(revision.into());
        Ok(())
    }
    pub async fn flush_embedding_receipts(&self, a: &crate::AuthorizedAccount) -> Result<bool> {
        let _guard = self.gate.read().await;
        let repository = self.account_repository(a).await?;
        let target = self.search_database.as_ref().ok_or(Error::Invalid)?;
        let identity = target.identity().await?;
        let receipts = target.completion_receipts(a.id()).await?;
        if receipts.is_empty() {
            return Ok(false);
        }
        let acknowledged = repository
            .accept_embedding_receipts(&identity, self.expected_search_model.as_deref(), &receipts)
            .await?;
        target.acknowledge_completion(a.id(), &acknowledged).await?;
        Ok(!acknowledged.is_empty())
    }
}
