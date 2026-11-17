//! Small indexed filter cohorts use bounded exact cosine retrieval. This avoids
//! losing isolated matching vectors outside an approximate graph neighborhood.
use super::*;
// The cohort is already bounded. Resolve its IDs through the integer primary
// key, instead of scanning a model/account index and discarding unrelated rows.
const SQLITE_VECTORS: &str = "SELECT id,message_id,embedding FROM chunks NOT INDEXED WHERE account_id=? AND model_revision=? AND id IN (SELECT value FROM json_each(?))";
const SQLITE_METADATA_COHORT: &str = "SELECT c.id FROM documents d INDEXED BY documents_filter_cohort CROSS JOIN chunks c INDEXED BY chunks_filter_cohort ON c.account_id=d.account_id AND c.message_id=d.message_id WHERE d.account_id=";
pub(super) async fn sqlite(
    db: &SqlitePool,
    account: &str,
    revision: &str,
    vector: &[f32],
    filter: &filter::CandidateFilter,
    selectors: &selectivity::Selectors,
    accelerator: &crate::search_vectors::VectorAccelerator,
) -> Result<Option<Vec<(i64, f32)>>> {
    if !filter.restricted() {
        return Ok(None);
    }
    let mut timing = crate::timing::SearchTiming::new("semantic_resident_filter");
    if accelerator
        .caught_up(db, account, revision)
        .await
        .unwrap_or(false)
    {
        use crate::search_vectors::FilterCohort;
        let cohort = accelerator
            .filter_cohort_vectors(account, revision, filter.clone())
            .await
            .unwrap_or(FilterCohort::Unavailable);
        if !matches!(cohort, FilterCohort::Unavailable)
            && accelerator
                .caught_up(db, account, revision)
                .await
                .unwrap_or(false)
        {
            match cohort {
                FilterCohort::Broad => return Ok(None),
                FilterCohort::Vectors(vectors) => return Ok(Some(rank(vector, vectors)?)),
                FilterCohort::Unavailable => unreachable!(),
            }
        }
    }
    timing.stage("semantic_filter_cohort");
    // Filter documents before expanding their chunks. Starting with an account's
    // chunks can repeat a global metadata rowid list for every vector.
    let metadata_candidates = [
        ("sender", filter.from.as_deref()),
        ("recipients", filter.to.as_deref()),
        ("subject", filter.subject.as_deref()),
    ]
    .iter()
    .any(|(column, value)| {
        value.is_some_and(|value| super::filter::sqlite_substring(column, value).is_some())
    });
    let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new("WITH cohort AS MATERIALIZED (");
    q.push(if metadata_candidates {
        SQLITE_METADATA_COHORT
    } else {
        "SELECT c.id FROM documents d INDEXED BY documents_filter_order CROSS JOIN chunks c INDEXED BY chunks_filter_cohort ON c.account_id=d.account_id AND c.message_id=d.message_id WHERE d.account_id="
    });
    q.push_bind(account)
        .push(" AND c.model_revision=")
        .push_bind(revision)
        .push(
            " AND c.source=d.source AND c.content_version=d.content_version AND d.state<>'deleted'",
        );
    for (column, value) in [
        ("sender", filter.from.as_deref()),
        ("recipients", filter.to.as_deref()),
        ("subject", filter.subject.as_deref()),
    ] {
        if let Some(value) = value {
            if let Some(needle) = selectors.literal(db, column, value).await? {
                q.push(
                    " AND d.rowid IN (SELECT rowid FROM metadata_terms WHERE metadata_terms MATCH ",
                )
                .push_bind(needle)
                .push(")");
            }
            q.push(" AND instr(d.")
                .push(column)
                .push(",")
                .push_bind(value)
                .push(")>0");
        }
    }
    for (operator, value) in [
        (">=", filter.since.as_deref()),
        ("<", filter.before.as_deref()),
    ] {
        if let Some(value) = value {
            q.push(" AND d.received_date")
                .push(operator)
                .push_bind(value);
        }
    }
    if let Some(seen) = filter.seen {
        q.push(" AND d.unread<>").push_bind(seen);
    }
    if filter.has_attachment {
        q.push(" AND json_array_length(d.attachment_details)>0");
    }
    if let Some(starred) = filter.starred {
        q.push(" AND d.starred=").push_bind(starred);
    }
    crate::classification::predicate!(q, true, "d.metadata", &filter.classification);
    if let Some(category) = filter.category.as_deref() {
        q.push(" AND (d.category IS NULL OR d.category=")
            .push_bind(category)
            .push(")");
    }
    if let Some(folders) = &filter.folders {
        q.push(" AND EXISTS(SELECT 1 FROM document_folders f WHERE f.account_id=d.account_id AND f.message_id=d.message_id AND f.folder IN (SELECT value FROM json_each(").push_bind(serde_json::to_string(folders)?).push(")))");
    }
    if filter.snoozed_only {
        q.push(" AND d.snoozed_until>").push_bind(&filter.at);
    }
    if filter.hide_snoozed {
        q.push(" AND (d.snoozed_until IS NULL OR d.snoozed_until<=")
            .push_bind(&filter.at)
            .push(")");
    }
    q.push(" LIMIT 4097) SELECT COALESCE(json_group_array(id),'[]') FROM cohort");
    timing.stage("semantic_filter_ids");
    // One bounded row avoids thousands of cross-thread SQLite row deliveries.
    let ids: String = {
        let mut read = budget::sqlite(db).await?;
        q.build_query_scalar().fetch_one(read.connection()).await?
    };
    let ids: Vec<i64> = serde_json::from_str(&ids)?;
    timing.stage("semantic_filter_vectors");
    if ids.len() > 4096 {
        return Ok(None);
    }
    if ids.is_empty() {
        return Ok(Some(vec![]));
    }
    let resident = if accelerator
        .caught_up(db, account, revision)
        .await
        .unwrap_or(false)
    {
        let vectors = accelerator
            .cohort_vectors(account, revision, &ids)
            .await
            .ok()
            .flatten();
        if vectors.is_some()
            && accelerator
                .caught_up(db, account, revision)
                .await
                .unwrap_or(false)
        {
            vectors
        } else {
            None
        }
    } else {
        None
    };
    let vectors = if let Some(vectors) = resident {
        vectors
    } else {
        sqlx::query_as::<_, (i64, String, Vec<u8>)>(SQLITE_VECTORS)
            .bind(account)
            .bind(revision)
            .bind(serde_json::to_string(&ids)?)
            .fetch_all(db)
            .await?
    };
    timing.stage("semantic_filter_cosine");
    Ok(Some(rank(vector, vectors)?))
}
fn rank(vector: &[f32], vectors: crate::search_vectors::CohortVectors) -> Result<Vec<(i64, f32)>> {
    let qnorm = vector
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    let mut hits = Vec::with_capacity(vectors.len());
    for (id, message, bytes) in vectors {
        if bytes.len() != 1536 {
            return Err(Error::Invalid);
        }
        let (mut dot, mut norm) = (0f64, 0f64);
        for (bytes, q) in bytes.chunks_exact(4).zip(vector) {
            let value = f64::from(f32::from_le_bytes(
                bytes.try_into().map_err(|_| Error::Invalid)?,
            ));
            if !value.is_finite() {
                return Err(Error::Invalid);
            }
            dot += value * f64::from(*q);
            norm += value * value;
        }
        if norm <= 0. {
            return Err(Error::Invalid);
        }
        hits.push((
            id,
            (1. - dot / (qnorm * norm.sqrt())).clamp(0., 2.) as f32,
            message,
        ));
    }
    hits.sort_by(|(a, ad, _), (b, bd, _)| ad.total_cmp(bd).then_with(|| a.cmp(b)));
    let mut seen = std::collections::HashSet::new();
    Ok(hits
        .into_iter()
        .filter_map(|(id, distance, message)| seen.insert(message).then_some((id, distance)))
        .take(200)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn metadata_cohorts_do_not_fetch_document_text_or_embedding_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap()
        else {
            unreachable!()
        };
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new("EXPLAIN QUERY PLAN ");
        q.push(SQLITE_METADATA_COHORT).push_bind("account")
            .push(" AND c.model_revision=").push_bind("revision")
            .push(" AND c.source=d.source AND c.content_version=d.content_version AND d.state<>'deleted' AND d.rowid IN (SELECT rowid FROM metadata_terms WHERE metadata_terms MATCH ")
            .push_bind("sender:\"abc\"")
            .push(") AND instr(d.sender,").push_bind("abc@example.invalid").push(")>0 LIMIT 4097");
        let plan: Vec<(i64, i64, i64, String)> = q.build_query_as().fetch_all(&db).await.unwrap();
        for index in ["documents_filter_cohort", "chunks_filter_cohort"] {
            assert!(
                plan.iter()
                    .any(|row| row.3.contains(&format!("USING COVERING INDEX {index}"))),
                "{plan:?}"
            );
        }
        assert!(
            plan.iter().any(|row| row
                .3
                .contains("documents_filter_cohort (account_id=? AND rowid=?)")),
            "{plan:?}"
        );
        assert!(
            !plan
                .iter()
                .any(|row| row.3.starts_with("SCAN d") || row.3.starts_with("SCAN c")),
            "{plan:?}"
        );
    }

    #[tokio::test]
    async fn flag_and_date_cohorts_use_compact_metadata_before_expanding_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap()
        else {
            unreachable!()
        };
        let sql="SELECT c.id FROM documents d INDEXED BY documents_filter_order CROSS JOIN chunks c INDEXED BY chunks_filter_cohort ON c.account_id=d.account_id AND c.message_id=d.message_id WHERE d.account_id=? AND c.model_revision=? AND c.source=d.source AND c.content_version=d.content_version AND d.state<>'deleted' AND d.unread=1 AND d.starred=0 AND d.received_date>=? LIMIT 4097";
        let plan: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(&format!("EXPLAIN QUERY PLAN {sql}"))
                .bind("account")
                .bind("revision")
                .bind("2026-01-01")
                .fetch_all(&db)
                .await
                .unwrap();
        for index in ["documents_filter_order", "chunks_filter_cohort"] {
            assert!(
                plan.iter()
                    .any(|row| row.3.starts_with("SEARCH ") && row.3.contains(index)),
                "{plan:?}"
            );
        }
        // SQLite does not label an index containing generated flags COVERING,
        // even when its bytecode reads every value from that index. Verify the
        // executed projection instead of weakening the body-free guarantee.
        let code = sqlx::query(&format!("EXPLAIN {sql}"))
            .bind("account")
            .bind("revision")
            .bind("2026-01-01")
            .fetch_all(&db)
            .await
            .unwrap();
        let root: i64 =
            sqlx::query_scalar("SELECT rootpage FROM sqlite_schema WHERE name='documents'")
                .fetch_one(&db)
                .await
                .unwrap();
        for row in &code {
            if row.get::<String, _>("opcode") == "OpenRead" && row.get::<i64, _>("p2") == root {
                let cursor = row.get::<i64, _>("p1");
                assert!(
                    !code
                        .iter()
                        .any(|op| op.get::<String, _>("opcode") == "Column"
                            && op.get::<i64, _>("p1") == cursor),
                    "document payload must remain unread"
                );
            }
        }
    }

    #[tokio::test]
    async fn bounded_exact_vectors_use_integer_primary_keys() {
        let dir = tempfile::tempdir().unwrap();
        let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap()
        else {
            unreachable!()
        };
        let plan: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(&format!("EXPLAIN QUERY PLAN {SQLITE_VECTORS}"))
                .bind("account")
                .bind("revision")
                .bind("[1,2,3]")
                .fetch_all(&db)
                .await
                .unwrap();
        assert!(
            plan.iter()
                .any(|row| row.3.contains("SEARCH chunks USING INTEGER PRIMARY KEY")),
            "{plan:?}"
        );
    }
}
