use super::*;

// Shared by every local query before LIMIT/keyset pagination. Overrides are
// materialized transactionally, so cached/provider projections use the same facts.
pub(crate) const CLASSIFICATION_SQL: &str = "
 AND (json_extract(cf.filter,'$.inbox_kind') IS NULL OR COALESCE(json_extract(m.document,'$.classification.kind'),'personal')=json_extract(cf.filter,'$.inbox_kind'))
 AND (json_extract(cf.filter,'$.email_type') IS NULL OR EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.types') t WHERE t.value=json_extract(cf.filter,'$.email_type')))
 AND (COALESCE(json_extract(cf.filter,'$.transactions'),0)=0 OR EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.types') t WHERE t.value IN ('receipt','invoice')))
 AND (COALESCE(json_extract(cf.filter,'$.attention'),0)=0 OR (julianday(json_extract(m.document,'$.received_at')) BETWEEN julianday('now','-30 days') AND julianday('now') AND (json_extract(m.document,'$.classification.attention_override')=1 OR (json_extract(m.document,'$.classification.attention_override') IS NULL AND EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.attention') a WHERE json_extract(a.value,'$.expires_at') IS NULL OR json_extract(a.value,'$.expires_at')>strftime('%Y-%m-%dT%H:%M:%fZ','now'))))))";
pub(crate) fn wire<T: serde::Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_value(v)?
        .as_str()
        .ok_or(Error::Invalid)?
        .into())
}
pub(crate) fn legacy_classification(category: &str) -> Classification {
    Classification {
        kind: if matches!(category, "people" | "other") {
            InboxKind::Personal
        } else {
            InboxKind::Automated
        },
        status: CategoryStatus::Ready,
        ..Default::default()
    }
}
impl Store {
    pub async fn requeue_classification_policy(
        &self,
        a: &AuthorizedAccount,
        revision: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .requeue_classification_policy(revision)
            .await
    }
    pub async fn request_classification(
        &self,
        a: &AuthorizedAccount,
        ids: &[String],
    ) -> Result<()> {
        if ids.len() > 200 {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .request_classification(ids)
            .await
    }
}

pub(crate) const CLASSIFICATION_PG_SQL: &str = "
 AND (cf.filter::jsonb->>'inbox_kind' IS NULL OR COALESCE(m.document::jsonb#>>'{classification,kind}','personal')=cf.filter::jsonb->>'inbox_kind')
 AND (cf.filter::jsonb->>'email_type' IS NULL OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(m.document::jsonb#>'{classification,types}') t WHERE t=cf.filter::jsonb->>'email_type'))
 AND (NOT COALESCE((cf.filter::jsonb->>'transactions')::BOOLEAN,FALSE) OR EXISTS(SELECT 1 FROM jsonb_array_elements_text(m.document::jsonb#>'{classification,types}') t WHERE t IN ('receipt','invoice')))
 AND (NOT COALESCE((cf.filter::jsonb->>'attention')::BOOLEAN,FALSE) OR ((m.document::jsonb->>'received_at')::TIMESTAMPTZ BETWEEN CURRENT_TIMESTAMP-INTERVAL '30 days' AND CURRENT_TIMESTAMP AND ((m.document::jsonb#>>'{classification,attention_override}')::BOOLEAN IS TRUE OR (m.document::jsonb#>>'{classification,attention_override}' IS NULL AND EXISTS(SELECT 1 FROM jsonb_array_elements(m.document::jsonb#>'{classification,attention}') a WHERE a->>'expires_at' IS NULL OR a->>'expires_at'>to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'))))))";

macro_rules! predicate {
    ($q:ident,$sqlite:expr,$document:expr,$filter:expr) => {
        if *$filter != dakia_domain::ClassificationFilter::default() {
            let sql = if $sqlite {
                crate::classification::CLASSIFICATION_SQL
            } else {
                crate::classification::CLASSIFICATION_PG_SQL
            };
            $q.push(" AND EXISTS(SELECT 1 FROM (SELECT ")
                .push_bind(serde_json::to_string($filter)?)
                .push(" AS filter) cf WHERE TRUE ")
                .push(sql.replace("m.document", $document))
                .push(")");
        }
    };
}
pub(crate) use predicate;
