//! Bounded authoritative metadata rehydration for disposable vector graphs.
use crate::search_backend::filter::CandidateMetadata;
use crate::{Error, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
#[derive(sqlx::FromRow)]
pub(super) struct Row {
    pub cursor: i64,
    pub id: i64,
    pub account: String,
    pub revision: String,
    pub bytes: Option<Vec<u8>>,
    pub metadata: Option<String>,
    pub message_id: Option<String>,
    pub sender: Option<String>,
    pub recipients: Option<String>,
    pub subject: Option<String>,
    pub snoozed_until: Option<String>,
    pub version: Option<i64>,
    pub has_attachment: Option<bool>,
    pub valid: bool,
    pub metadata_only: bool,
}
pub(super) async fn rows(db: &sqlx::SqlitePool, cursor: i64, replay: bool) -> Result<Vec<Row>> {
    let base = if replay {
        "SELECT seq AS cursor,chunk_id AS id,account_id,model_revision,metadata_only FROM embedding_changes WHERE seq>? ORDER BY seq LIMIT 256"
    } else {
        "SELECT id AS cursor,id,account_id,model_revision,0 AS metadata_only FROM chunks WHERE id>? ORDER BY id LIMIT 256"
    };
    let sql=format!("WITH base AS MATERIALIZED ({base}), sizes AS (SELECT b.*,COALESCE(length(c.embedding),0)+COALESCE(length(CAST(d.metadata AS BLOB)),0)+COALESCE(length(CAST(d.sender AS BLOB)),0)+COALESCE(length(CAST(d.recipients AS BLOB)),0)+COALESCE(length(CAST(d.subject AS BLOB)),0) AS bytes_count FROM base b LEFT JOIN chunks c ON c.id=b.id AND c.account_id=b.account_id AND c.model_revision=b.model_revision LEFT JOIN documents d ON d.account_id=c.account_id AND d.message_id=c.message_id), budget AS (SELECT *,sum(bytes_count) OVER(ORDER BY cursor) AS total_bytes FROM sizes), selected AS MATERIALIZED (SELECT * FROM budget WHERE total_bytes<=8388608 OR cursor=(SELECT min(cursor) FROM budget)) SELECT b.cursor,b.id,b.account_id AS account,b.model_revision AS revision,b.metadata_only,c.embedding AS bytes,d.metadata,d.message_id,d.sender,d.recipients,d.subject,d.snoozed_until,d.filter_revision AS version,(json_array_length(d.attachment_details)>0) AS has_attachment,(c.id IS NOT NULL AND d.source=c.source AND d.content_version=c.content_version AND d.state<>'deleted') AS valid FROM selected b LEFT JOIN chunks c ON c.id=b.id AND c.account_id=b.account_id AND c.model_revision=b.model_revision LEFT JOIN documents d ON d.account_id=c.account_id AND d.message_id=c.message_id ORDER BY b.cursor");
    Ok(sqlx::query_as(&sql).bind(cursor).fetch_all(db).await?)
}
#[derive(Default)]
pub(super) struct Metadata {
    pub entries: BTreeMap<u64, Arc<CandidateMetadata>>,
    pub literals: super::literals::Literals,
    documents: BTreeMap<(String, String), (i64, Weak<CandidateMetadata>)>,
}
impl Metadata {
    fn remove(&mut self, key: u64) {
        if let Some(old) = self.entries.remove(&key) {
            self.literals.remove(key, &old);
            if Arc::strong_count(&old) == 1 {
                let identity = (old.message.account_id.clone(), old.message.id.clone());
                if self
                    .documents
                    .get(&identity)
                    .is_some_and(|(_, cached)| cached.ptr_eq(&Arc::downgrade(&old)))
                {
                    self.documents.remove(&identity);
                }
            }
        }
    }
    pub fn apply(&mut self, row: &Row) -> Result<()> {
        let key = u64::try_from(row.id).map_err(|_| Error::Invalid)?;
        if !row.valid {
            self.remove(key);
            return Ok(());
        }
        let identity = (
            row.account.clone(),
            row.message_id.clone().ok_or(Error::Invalid)?,
        );
        let version = row.version.ok_or(Error::Invalid)?;
        let cached = self
            .documents
            .get(&identity)
            .filter(|(v, _)| *v == version)
            .and_then(|(_, cached)| cached.upgrade());
        let metadata = if let Some(cached) = cached {
            cached
        } else {
            let message: crate::Message =
                serde_json::from_str(row.metadata.as_deref().ok_or(Error::Invalid)?)?;
            if message.account_id != identity.0 || message.id != identity.1 {
                return Err(Error::Invalid);
            }
            Arc::new(CandidateMetadata {
                message,
                sender: row.sender.clone().ok_or(Error::Invalid)?,
                recipients: row.recipients.clone().ok_or(Error::Invalid)?,
                subject: row.subject.clone().ok_or(Error::Invalid)?,
                snoozed_until: row.snoozed_until.clone(),
                has_attachment: row.has_attachment.unwrap_or(false),
            })
        };
        self.remove(key);
        self.documents
            .insert(identity, (version, Arc::downgrade(&metadata)));
        self.literals.insert(key, &metadata);
        self.entries.insert(key, metadata);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn metadata_pages_bound_payload_before_materializing_large_participant_fields() {
        let dir = tempfile::tempdir().unwrap();
        let target = crate::search_backend::SearchDatabase::open(dir.path(), None)
            .await
            .unwrap();
        let message:crate::Message=serde_json::from_value(serde_json::json!({"id":"large-meta","account_id":"metadata-account","thread_id":"large-meta","provider_id":"source","folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"Metadata","preview":"","received_at":"2026-01-01T00:00:00Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":1})).unwrap();
        target
            .deliver(
                "metadata-account",
                "large-meta",
                1,
                Some(crate::search_backend::Document {
                    message,
                    source: "source".into(),
                    sender: "fixture".into(),
                    recipients: "word ".repeat(100000),
                    mime_recipients: "".into(),
                    attachment_details: "[]".into(),
                    body_payload: true,
                    snoozed_until: None,
                    subject: "metadata".into(),
                    body: "passage".into(),
                    participants: "".into(),
                    attachments: "".into(),
                    state: "ready".into(),
                }),
            )
            .await
            .unwrap();
        let crate::search_backend::SearchDatabase::Sqlite(db) = target else {
            unreachable!()
        };
        let mut vector = vec![0f32; 384];
        vector[0] = 1.;
        let bytes = vector
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        sqlx::query("WITH RECURSIVE positions(i) AS(SELECT 0 UNION ALL SELECT i+1 FROM positions WHERE i<39) INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT 'metadata-account','large-meta','source','metadata-fixture',i,'passage',?,1,1 FROM positions").bind(bytes).execute(&db).await.unwrap();
        let mut cursor = 0;
        let mut total = 0;
        loop {
            let page = rows(&db, cursor, false).await.unwrap();
            if page.is_empty() {
                break;
            }
            assert!(page.len() < 40);
            let bytes: usize = page
                .iter()
                .map(|row| {
                    row.bytes.as_ref().map_or(0, Vec::len)
                        + row.metadata.as_ref().map_or(0, String::len)
                        + row.sender.as_ref().map_or(0, String::len)
                        + row.recipients.as_ref().map_or(0, String::len)
                        + row.subject.as_ref().map_or(0, String::len)
                })
                .sum();
            assert!(
                bytes <= 8388608,
                "page exceeded its text/vector payload budget"
            );
            cursor = page.last().unwrap().cursor;
            total += page.len();
        }
        assert_eq!(total, 40);
    }
}
