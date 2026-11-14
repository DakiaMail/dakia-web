//! Strict phrase verification over an already bounded candidate message set.
//! Each phrase may match any valid chunk; no global posting-list scan is needed.
use super::*;

pub(super) async fn ids(
    db: &SearchDatabase,
    account: &str,
    ids: &str,
    phrases: &[String],
) -> Result<std::collections::HashSet<String>> {
    if phrases.len() > 2048 {
        return Err(Error::Invalid);
    }
    let canonical = phrases
        .iter()
        .map(|phrase| crate::lexical_text::unit(phrase)?.ok_or(Error::Invalid))
        .collect::<Result<Vec<_>>>()?;
    macro_rules! run {
        ($kind:ty,$pool:expr,$sqlite:expr,$bounded:path) => {{
            let mut q = sqlx::QueryBuilder::<$kind>::new(if $sqlite {
                "SELECT d.message_id FROM documents d INDEXED BY documents_filter_cohort WHERE d.account_id="
            } else { "SELECT d.message_id FROM documents d WHERE d.account_id=" });
            q.push_bind(account).push(" AND d.state<>'deleted' AND ");
            if $sqlite {
                // Resolve the bounded message identities through the unique key,
                // then verify versions/state on compact account-scoped metadata.
                q.push("d.rowid IN (SELECT rowid FROM documents WHERE account_id=").push_bind(account)
                    .push(" AND message_id IN (SELECT value FROM json_each(").push_bind(ids).push(")))");
            } else {
                q.push("d.message_id IN (SELECT jsonb_array_elements_text(").push_bind(ids).push("::jsonb))");
            }
            for phrase in &canonical {
                q.push(if $sqlite { " AND EXISTS(SELECT 1 FROM lexical_chunks c INDEXED BY lexical_message CROSS JOIN lexical_accounts a ON a.account_id=c.account_id WHERE c.account_id=d.account_id AND c.message_id=d.message_id AND c.content_version=d.content_version AND c.revision IN (4,5) AND a.revision=" } else { " AND EXISTS(SELECT 1 FROM lexical_chunks c JOIN lexical_accounts a ON a.account_id=c.account_id WHERE c.account_id=d.account_id AND c.message_id=d.message_id AND c.content_version=d.content_version AND c.revision IN (4,5) AND a.revision=" })
                    .push_bind(crate::lexical_text::REVISION).push(" AND ");
                if $sqlite {
                    // A point read of bounded candidate chunks is cheaper than
                    // initializing the same dense FTS phrase cursor per chunk.
                    q.push("instr(' '||c.text||' ',")
                        .push_bind(format!(" {phrase} ")).push(")>0)");
                } else {
                    q.push("((c.phrase_safe AND c.lexemes @@ to_tsquery('simple',")
                        .push_bind(probe::postgres_phrase(phrase,None)).push(")) OR (NOT c.phrase_safe AND strpos(' '||c.text||' ',")
                        .push_bind(format!(" {phrase} ")).push(")>0)))");
                }
            }
            let mut read=$bounded($pool).await?;
            q.build_query_scalar::<String>().fetch_all(read.connection()).await?
        }};
    }
    let rows = match db {
        SearchDatabase::Sqlite(pool) => run!(sqlx::Sqlite, pool, true, budget::sqlite),
        SearchDatabase::Postgres(pool) => run!(sqlx::Postgres, pool, false, budget::postgres),
    };
    Ok(rows.into_iter().collect())
}
