//! Exact source-side proof that the current published message finished all
//! semantic stages. Receipts are disposable and invalidated by mailbox writes.
use super::*;
use crate::search_backend::completion::CompletionReceipt;
impl AccountRepository {
    pub(crate) async fn semantic_profile(&self, revision: &str, available: bool) -> Result<()> {
        scoped!(self, scope, true, {
            let previous: Option<(String, i64)> = tuples!(
                scope,
                "SELECT model_revision,available FROM search_semantic_profile WHERE id=1",
                [],
                "SELECT model_revision,available FROM search_semantic_profile WHERE account_id=$1",
                [&scope.account],
                fetch_optional
            );
            write!(scope,"INSERT INTO search_semantic_profile(id,model_revision,available,lexical_revision) VALUES(1,?,?,?) ON CONFLICT(id) DO UPDATE SET model_revision=excluded.model_revision,available=excluded.available,lexical_revision=MAX(search_semantic_profile.lexical_revision,excluded.lexical_revision)",[revision,i64::from(available),crate::lexical_text::REVISION],"INSERT INTO search_semantic_profile(account_id,model_revision,available,lexical_revision) VALUES($1,$2,$3,$4) ON CONFLICT(account_id) DO UPDATE SET model_revision=excluded.model_revision,available=excluded.available,lexical_revision=GREATEST(search_semantic_profile.lexical_revision,excluded.lexical_revision)",[&scope.account,revision,i64::from(available),crate::lexical_text::REVISION]);
            if previous
                .as_ref()
                .is_none_or(|(r, a)| r != revision || *a != i64::from(available))
            {
                scope.event(0, "index_changed", "").await?;
            }
            Ok::<_, Error>(())
        })
    }
    /// Backfill only a bounded publication page for installations which predate
    /// delivery versions. Metadata delivery preserves retained target vectors.
    pub(crate) async fn seed_search_versions(&self) -> Result<()> {
        scoped!(self, scope, true, {
            let ids:Vec<String>=value!(scope,"WITH versions AS (SELECT message_id FROM search_documents WHERE delivered_version=0 AND delivered=1 ORDER BY message_id LIMIT 200), stages AS (SELECT message_id FROM search_semantic_receipts WHERE lexical_revision<? ORDER BY message_id LIMIT 200), upgrade AS (SELECT message_id FROM versions UNION SELECT message_id FROM stages ORDER BY message_id LIMIT 200) SELECT d.message_id FROM upgrade d WHERE NOT EXISTS(SELECT 1 FROM search_index_outbox o WHERE o.message_id=d.message_id) AND NOT EXISTS(SELECT 1 FROM search_dirty x WHERE x.message_id=d.message_id)",[crate::lexical_text::REVISION],"WITH versions AS MATERIALIZED (SELECT message_id FROM search_documents WHERE account_id=$1 AND delivered_version=0 AND delivered=1 ORDER BY message_id COLLATE \"C\" LIMIT 200), stages AS MATERIALIZED (SELECT message_id FROM search_semantic_receipts WHERE account_id=$1 AND lexical_revision<$2 ORDER BY message_id COLLATE \"C\" LIMIT 200), upgrade AS MATERIALIZED (SELECT message_id FROM versions UNION SELECT message_id FROM stages ORDER BY message_id LIMIT 200) SELECT d.message_id FROM upgrade d WHERE NOT EXISTS(SELECT 1 FROM search_index_outbox o WHERE o.account_id=$1 AND o.message_id=d.message_id) AND NOT EXISTS(SELECT 1 FROM search_dirty x WHERE x.account_id=$1 AND x.message_id=d.message_id)",[&scope.account,crate::lexical_text::REVISION],fetch_all);
            for id in ids {
                write!(scope,"UPDATE search_index_clock SET value=value+1 WHERE id=1",[],"UPDATE account_locks SET search_sequence=search_sequence+1 WHERE account_id=$1",[&scope.account]);
                write!(scope,"INSERT INTO search_index_outbox(message_id,sequence) SELECT ?,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence",[&id],"INSERT INTO search_index_outbox(account_id,message_id,sequence) SELECT account_id,$2,search_sequence FROM account_locks WHERE account_id=$1 ON CONFLICT(account_id,message_id) DO UPDATE SET sequence=excluded.sequence",[&scope.account,&id]);
            }
            Ok::<_, Error>(())
        })
    }
    pub(crate) async fn accept_embedding_receipts(
        &self,
        identity: &str,
        expected: Option<&str>,
        receipts: &[CompletionReceipt],
    ) -> Result<Vec<CompletionReceipt>> {
        if receipts.len() > 200 {
            return Err(Error::Invalid);
        }
        scoped!(self, scope, true, {
            let current_identity:Option<String>=value!(scope,"SELECT document FROM provider_documents WHERE key='search:database'",[],"SELECT document FROM provider_documents WHERE account_id=$1 AND key='search:database'",[&scope.account],fetch_optional);
            let profile: Option<String> = value!(
                scope,
                "SELECT model_revision FROM search_semantic_profile WHERE id=1",
                [],
                "SELECT model_revision FROM search_semantic_profile WHERE account_id=$1",
                [&scope.account],
                fetch_optional
            );
            let Some(revision) = expected.or(profile.as_deref()) else {
                return Ok(vec![]);
            };
            let mut acknowledged = vec![];
            let ids = serde_json::to_string(
                &receipts
                    .iter()
                    .map(|r| r.message_id.as_str())
                    .collect::<Vec<_>>(),
            )?;
            type SourceRow = (String, String, i64, i64, String, bool, bool, bool);
            let current:Vec<SourceRow>=tuples!(scope,"SELECT d.message_id,d.source,d.delivered,d.delivered_version,d.body_state,EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.id=d.message_id),EXISTS(SELECT 1 FROM search_dirty x WHERE x.message_id=d.message_id),EXISTS(SELECT 1 FROM search_index_outbox o WHERE o.message_id=d.message_id) FROM search_documents d WHERE d.message_id IN (SELECT value FROM json_each(?))",[&ids],"SELECT d.message_id,d.source,d.delivered,d.delivered_version,d.body_state,EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=d.account_id AND e.id=d.message_id),EXISTS(SELECT 1 FROM search_dirty x WHERE x.account_id=d.account_id AND x.message_id=d.message_id),EXISTS(SELECT 1 FROM search_index_outbox o WHERE o.account_id=d.account_id AND o.message_id=d.message_id) FROM search_documents d WHERE d.account_id=$1 AND d.message_id IN (SELECT jsonb_array_elements_text($2::jsonb))",[&scope.account,&ids],fetch_all);
            let current: HashMap<_, _> = current
                .into_iter()
                .map(
                    |(id, source, delivered, version, state, eligible, dirty, pending)| {
                        (
                            id,
                            (source, delivered, version, state, eligible, dirty, pending),
                        )
                    },
                )
                .collect();
            let mut valid = vec![];
            for receipt in receipts {
                // A newer lexical writer may still need this durable receipt.
                // Older binaries must leave it for that writer to acknowledge.
                if receipt.lexical_revision > crate::lexical_text::REVISION {
                    continue;
                }
                if current_identity.as_deref().is_some_and(|i| i != identity)
                    || receipt.model_revision != revision
                    || receipt.lexical_revision != crate::lexical_text::REVISION
                {
                    acknowledged.push(receipt.clone());
                    continue;
                }
                if current_identity.is_none() || profile.as_deref() != Some(revision) {
                    continue;
                }
                let Some((source, delivered, version, state, eligible, dirty, pending)) =
                    current.get(&receipt.message_id)
                else {
                    acknowledged.push(receipt.clone());
                    continue;
                };
                if source != &receipt.source || !eligible || *version > receipt.version {
                    acknowledged.push(receipt.clone());
                    continue;
                }
                if *delivered != 1
                    || *version < receipt.version
                    || *dirty
                    || *pending
                    || matches!(state.as_str(), "pending" | "fetch_error" | "quota")
                {
                    continue;
                }
                valid.push(receipt);
                acknowledged.push(receipt.clone());
            }
            if !valid.is_empty() {
                let account = scope.account.clone();
                macro_rules! insert {($kind:ty,$db:expr,$local:expr)=>{{
                    let mut query=sqlx::QueryBuilder::<$kind>::new(if $local {"INSERT INTO search_semantic_receipts(message_id,version,source,model_revision,lexical_revision) "}else{"INSERT INTO search_semantic_receipts(account_id,message_id,version,source,model_revision,lexical_revision) "});
                    query.push_values(valid.iter(),|mut values,receipt|{
                        if !$local {values.push_bind(&account);}
                        values.push_bind(&receipt.message_id).push_bind(receipt.version).push_bind(&receipt.source).push_bind(&receipt.model_revision).push_bind(receipt.lexical_revision);
                    });
                    query.push(if $local {" ON CONFLICT(message_id)"}else{" ON CONFLICT(account_id,message_id)"}).push(" DO UPDATE SET version=excluded.version,source=excluded.source,model_revision=excluded.model_revision,lexical_revision=excluded.lexical_revision WHERE search_semantic_receipts.lexical_revision<>excluded.lexical_revision OR search_semantic_receipts.version<>excluded.version OR search_semantic_receipts.source<>excluded.source OR search_semantic_receipts.model_revision<>excluded.model_revision");
                    query.build().execute(&mut **$db).await?;
                }};}
                match &mut scope.connection {
                    Connection::Sqlite(db) => insert!(sqlx::Sqlite, db, true),
                    Connection::Postgres(db) => insert!(sqlx::Postgres, db, false),
                }
            }
            Ok::<_, Error>(acknowledged)
        })
    }
}
