//! Vector retrieval only. Shared ranking and current application authorization
//! remain above this repository; every accelerator hit is rechecked in SQL.
use super::*;

#[derive(Debug)]
pub struct SemanticMatch {
    pub message: Message,
    pub distance: f64,
    pub excerpt: String,
    /// 0: participants/subject, 1: body, 2: attachment text.
    pub field: i64,
    pub attachment: Option<String>,
}
pub(crate) struct VectorRow {
    pub message_id: String,
    pub distance: f64,
    pub excerpt: String,
    pub field: i64,
    pub attachment: Option<String>,
    pub source: String,
    pub sender: String,
    pub recipients: String,
    pub subject: String,
}
type StoredRow = (
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    i64,
    i64,
);
type DistanceRow = (
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    i64,
    i64,
    f64,
);
// Candidate IDs already bound this work. Without NOT INDEXED, SQLite can
// prefer an account index and scan every vector before testing the small ID
// set. Integer primary-key lookups remain available with NOT INDEXED.
const SQLITE_PAYLOADS: &str = "SELECT COALESCE(json_group_array(json_array(c.id,d.message_id,d.source,d.sender,d.recipients,d.subject,c.text,c.field,CASE WHEN c.field=2 THEN attachments.attachment_details ELSE '[]' END,c.byte_start,c.byte_end)),'[]') FROM chunks c NOT INDEXED CROSS JOIN documents identity ON identity.account_id=c.account_id AND identity.message_id=c.message_id CROSS JOIN documents d INDEXED BY documents_filter_cohort ON d.account_id=identity.account_id AND d.rowid=identity.rowid LEFT JOIN documents attachments ON c.field=2 AND attachments.account_id=d.account_id AND attachments.rowid=d.rowid WHERE c.id IN (SELECT value FROM json_each(?)) AND c.account_id=? AND c.model_revision=? AND c.content_version=d.content_version AND c.source=d.source AND d.state<>'deleted'";
impl SearchConnection {
    pub async fn vector_rows(
        &self,
        account: &str,
        revision: &str,
        vector: &[f32],
        filter: &CachedSearchFilter<'_>,
    ) -> Result<Vec<VectorRow>> {
        if vector.len() != 384
            || vector.iter().any(|v| !v.is_finite())
            || vector.iter().map(|v| v * v).sum::<f32>() < 0.01
        {
            return Err(Error::Invalid);
        }
        let db = self.semantic_connection().await?;
        let rows: Vec<(StoredRow, f64)> = match db {
            SearchDatabase::Sqlite(db) => {
                let mut timing = crate::timing::SearchTiming::new("semantic_filter");
                let predicate = super::filter::CandidateFilter::from(filter);
                let selective = super::filter_exact::sqlite(
                    &db,
                    account,
                    revision,
                    vector,
                    &predicate,
                    &self.selectors,
                    &self.vectors,
                )
                .await?;
                let exact = selective.is_some();
                timing.stage("semantic_accelerator_proof");
                let hits = if let Some(hits) = selective {
                    hits
                } else {
                    if !self.vectors.caught_up(&db, account, revision).await? {
                        return Err(Error::Conflict);
                    }
                    timing.stage("semantic_ann");
                    let hits = self
                        .vectors
                        .message_candidates(account, revision, vector, predicate.clone())
                        .await?;
                    if predicate.restricted() && hits.is_empty() {
                        return Err(Error::Conflict);
                    }
                    hits
                };
                if hits.is_empty() {
                    return Ok(vec![]);
                }
                let ids =
                    serde_json::to_string(&hits.iter().map(|(id, _)| id).collect::<Vec<_>>())?;
                timing.stage("semantic_payloads");
                let batch: String = sqlx::query_scalar(SQLITE_PAYLOADS)
                    .bind(ids)
                    .bind(account)
                    .bind(revision)
                    .fetch_one(&db)
                    .await?;
                let rows: Vec<StoredRow> = serde_json::from_str(&batch)?;
                timing.stage("semantic_accelerator_recheck");
                if !exact && !self.vectors.caught_up(&db, account, revision).await? {
                    return Err(Error::Conflict);
                }
                let mut rows: std::collections::HashMap<_, _> =
                    rows.into_iter().map(|row| (row.0, row)).collect();
                timing.stage("semantic_candidate_results");
                hits.into_iter()
                    .filter_map(|(id, distance)| {
                        rows.remove(&id).map(|row| (row, f64::from(distance)))
                    })
                    .collect()
            }
            SearchDatabase::Postgres(db) => {
                let vector = serde_json::to_string(vector)?;
                let mut read = budget::postgres(&db).await?;
                // Filtering after ANN needs iterative scans. Bound expansion so
                // selective filters cannot turn a request into a mailbox scan.
                sqlx::query("SET LOCAL hnsw.iterative_scan='strict_order'")
                    .execute(read.connection())
                    .await?;
                sqlx::query("SET LOCAL hnsw.ef_search=128")
                    .execute(read.connection())
                    .await?;
                sqlx::query("SET LOCAL hnsw.max_scan_tuples=20000")
                    .execute(read.connection())
                    .await?;
                let filter = super::filter::CandidateFilter::from(filter);
                let mut rows = Vec::new();
                for exact in [filter.restricted(), true] {
                    let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new(
                        "WITH query_vector AS MATERIALIZED (SELECT ",
                    );
                    // Generic prepared plans must parse the vector once, not once per passage.
                    q.push_bind(&vector).push("::text::vector AS vector), ");
                    if exact {
                        // Evaluate strict predicates once per email, before expanding its passages.
                        q.push("eligible AS MATERIALIZED (SELECT d.account_id,d.message_id,d.source,d.content_version FROM documents d WHERE d.account_id=")
                            .push_bind(account).push(" AND d.state<>'deleted'");
                    } else {
                        q.push("candidates AS MATERIALIZED (SELECT c.id,c.message_id,c.embedding<=>(SELECT vector FROM query_vector) AS distance FROM chunks c WHERE c.account_id=")
                            .push_bind(account).push(" AND c.model_revision=").push_bind(revision)
                            .push(" AND EXISTS(SELECT 1 FROM documents d WHERE d.account_id=c.account_id AND d.message_id=c.message_id AND c.content_version=d.content_version AND c.source=d.source AND d.state<>'deleted'");
                    }
                    for (column, value) in [
                        ("sender", filter.from.as_deref()),
                        ("recipients", filter.to.as_deref()),
                        ("subject", filter.subject.as_deref()),
                    ] {
                        if let Some(value) = value {
                            q.push(" AND d.")
                                .push(column)
                                .push(" LIKE ")
                                .push_bind(super::filter::postgres_substring(value))
                                .push(" ESCAPE '!'");
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
                    if let Some(starred) = filter.starred {
                        q.push(" AND d.starred=").push_bind(starred);
                    }
                    crate::classification::predicate!(
                        q,
                        false,
                        "d.metadata",
                        &filter.classification
                    );
                    if let Some(category) = filter.category.as_deref() {
                        q.push(" AND (d.category IS NULL OR d.category=")
                            .push_bind(category)
                            .push(")");
                    }
                    if let Some(folders) = &filter.folders {
                        q.push(" AND EXISTS(SELECT 1 FROM document_folders f WHERE f.account_id=d.account_id AND f.message_id=d.message_id AND f.folder=ANY(").push_bind(folders).push("))");
                    }
                    if filter.snoozed_only {
                        q.push(" AND d.snoozed_until>").push_bind(&filter.at);
                    }
                    if filter.hide_snoozed {
                        q.push(" AND (d.snoozed_until IS NULL OR d.snoozed_until<=")
                            .push_bind(&filter.at)
                            .push(")");
                    }
                    if exact {
                        q.push("), candidates AS MATERIALIZED (SELECT DISTINCT ON(c.message_id) c.id,c.message_id,c.embedding<=>(SELECT vector FROM query_vector) AS distance FROM eligible d JOIN LATERAL(SELECT c.id,c.message_id,c.embedding FROM chunks c WHERE c.account_id=d.account_id AND c.message_id=d.message_id AND c.model_revision=")
                            .push_bind(revision).push(" AND c.content_version=d.content_version AND c.source=d.source OFFSET 0) c ON TRUE ORDER BY c.message_id,(c.embedding<=>(SELECT vector FROM query_vector))+0,c.id");
                    } else {
                        q.push(" OFFSET 0) ORDER BY c.embedding<=>(SELECT vector FROM query_vector) LIMIT 1024");
                    }
                    q.push("), best AS (SELECT DISTINCT ON(message_id) id,message_id,distance FROM candidates ORDER BY message_id,distance,id), head AS MATERIALIZED (SELECT id,distance FROM best ORDER BY distance,id LIMIT 200) SELECT c.id,d.message_id,d.source,d.sender,d.recipients,d.subject,c.text,c.field,CASE WHEN c.field=2 THEN d.attachment_details ELSE '[]' END,c.byte_start,c.byte_end,b.distance FROM head b JOIN chunks c ON c.id=b.id JOIN documents d ON d.account_id=c.account_id AND d.message_id=c.message_id ORDER BY b.distance,b.id LIMIT 200");
                    rows = q
                        .build_query_as::<DistanceRow>()
                        .fetch_all(read.connection())
                        .await?;
                    // A long message must not consume the entire passage shortlist.
                    if exact || rows.len() == 200 {
                        break;
                    }
                }
                rows.into_iter()
                    .map(
                        |(id, m, src, s, r, sub, text, field, details, start, end, d)| {
                            ((id, m, src, s, r, sub, text, field, details, start, end), d)
                        },
                    )
                    .collect()
            }
        };
        rows.into_iter()
            .map(
                |(
                    (
                        _,
                        message_id,
                        source,
                        sender,
                        recipients,
                        subject,
                        excerpt,
                        field,
                        details,
                        start,
                        end,
                    ),
                    distance,
                )| {
                    let (excerpt, attachment) =
                        attachment_excerpt(excerpt, field, &details, start, end);
                    Ok(VectorRow {
                        message_id,
                        distance,
                        excerpt,
                        field,
                        attachment,
                        source,
                        sender,
                        recipients,
                        subject,
                    })
                },
            )
            .collect()
    }
}

/// Legacy payloads without trustworthy byte ranges retain their passage without
/// inventing an attachment identity. Details describe normalized UTF-8 text.
fn attachment_excerpt(
    text: String,
    field: i64,
    details: &str,
    start: i64,
    end: i64,
) -> (String, Option<String>) {
    if field != 2 || start < 0 || end <= start {
        return (text, None);
    }
    let Ok(parts) = serde_json::from_str::<Vec<serde_json::Value>>(details) else {
        return (text, None);
    };
    let selected = parts
        .iter()
        .filter_map(|part| {
            let from = part.get("byte_start")?.as_i64()?;
            let until = part.get("byte_end")?.as_i64()?;
            if from < 0 || until <= from {
                return None;
            }
            let a = from.max(start);
            let b = until.min(end);
            let name = part.get("filename")?.as_str()?;
            (a < b).then_some((b - a, a, b, name))
        })
        .max_by_key(|(overlap, _, _, _)| *overlap);
    let Some((_, a, b, name)) = selected else {
        return (text, None);
    };
    let range = (a - start) as usize..(b - start) as usize;
    let Some(excerpt) = text.get(range) else {
        return (text, None);
    };
    (excerpt.to_owned(), Some(name.to_owned()))
}

impl crate::Store {
    /// Returns best passage per message, after current ownership, identity,
    /// exclusions and strict structured filters are validated in application
    /// storage. Free text relevance and phrase constraints belong to fusion.
    pub async fn semantic_search_rows(
        &self,
        a: &crate::AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
        revision: &str,
        vector: &[f32],
    ) -> Result<Vec<SemanticMatch>> {
        self.semantic_search_rows_with_phrases(a, filter, revision, vector, &[])
            .await
    }
    /// Independent application-state and target-phrase reads overlap. Both must
    /// succeed before any semantic candidate can be admitted to the result.
    pub async fn semantic_search_rows_with_phrases(
        &self,
        a: &crate::AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
        revision: &str,
        vector: &[f32],
        phrases: &[String],
    ) -> Result<Vec<SemanticMatch>> {
        let mut timing = crate::timing::SearchTiming::new("semantic_admission");
        let _guard = self.gate.read().await;
        let repository = self.account_repository(a).await?;
        let target = self.search_database.as_ref().ok_or(Error::Invalid)?;
        timing.stage("semantic_retrieval");
        let candidates = target.vector_rows(a.id(), revision, vector, filter).await?;
        timing.stage("semantic_validation");
        let ids: std::collections::BTreeSet<_> =
            candidates.iter().map(|c| c.message_id.clone()).collect();
        let ids: Vec<_> = ids.into_iter().collect();
        let validate = async {
            let mut timing = crate::timing::SearchTiming::new("semantic_source_exclusions");
            let excluded = self.search_excluded_folders(a).await?;
            timing.stage("semantic_source_validation");
            let mut current = std::collections::HashMap::new();
            // Chunk retrieval and application validation retain independent
            // bounded pages, including long messages with many passages.
            for ids in ids.chunks(200) {
                for (message, snoozed) in repository.current_search_messages(ids).await? {
                    current.insert(message.id.clone(), (message, snoozed));
                }
            }
            Ok::<_, Error>((excluded, current))
        };
        let constrain = async {
            if phrases.is_empty() {
                return Ok::<_, Error>(None);
            }
            let _timing = crate::timing::SearchTiming::new("semantic_phrase_constraints");
            let mut allowed = std::collections::HashSet::new();
            for ids in ids.chunks(200) {
                allowed.extend(target.phrase_ids(a.id(), ids, phrases).await?);
            }
            Ok(Some(allowed))
        };
        let ((excluded, mut current), allowed) = tokio::try_join!(validate, constrain)?;
        let mut result = Vec::new();
        for row in candidates {
            if allowed
                .as_ref()
                .is_some_and(|ids| !ids.contains(&row.message_id))
            {
                continue;
            }
            if [
                (filter.from, row.sender.as_str()),
                (filter.to, row.recipients.as_str()),
                (filter.subject, row.subject.as_str()),
            ]
            .into_iter()
            .any(|(value, field)| value.is_some_and(|v| !field.contains(&normalize_search(v))))
            {
                continue;
            }
            let Some((message, snoozed)) = current.get(&row.message_id) else {
                continue;
            };
            if !filter.classification.matches(message, &crate::now())
                || !filter.matches_date(message)
                || message.provider_id != row.source
                || message.folder_ids.is_empty()
                || message
                    .folder_ids
                    .iter()
                    .any(|f| excluded.iter().any(|e| e.eq_ignore_ascii_case(f)))
                || filter.seen.is_some_and(|seen| message.unread == seen)
                || filter.starred.is_some_and(|s| message.starred != s)
                || filter
                    .folders
                    .is_some_and(|fs| !message.folder_ids.iter().any(|f| fs.contains(f)))
                || filter
                    .category
                    .is_some_and(|c| message.category.as_ref().is_some_and(|v| v != c))
                || (filter.snoozed_only && !snoozed)
                || (filter.hide_snoozed && *snoozed)
            {
                continue;
            }
            let (message, _) = current.remove(&row.message_id).ok_or(Error::Invalid)?;
            result.push(SemanticMatch {
                message,
                distance: row.distance,
                excerpt: row.excerpt,
                field: row.field,
                attachment: row.attachment,
            });
            if result.len() == 200 {
                break;
            }
        }
        Ok(result)
    }
}
impl crate::Store {
    /// Quoted text is a strict constraint even for semantically retrieved hits.
    /// Candidate restriction is pushed into both native lexical repositories.
    pub async fn constrain_search_phrases(
        &self,
        a: &crate::AuthorizedAccount,
        mut matches: Vec<SemanticMatch>,
        phrases: &[String],
    ) -> Result<Vec<SemanticMatch>> {
        if phrases.is_empty() {
            return Ok(matches);
        }
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        let target = self.search_database.as_ref().ok_or(Error::Invalid)?;
        let ids: Vec<_> = matches.iter().map(|m| m.message.id.clone()).collect();
        let _timing = crate::timing::SearchTiming::new("semantic_phrase_constraints");
        let mut allowed = std::collections::HashSet::new();
        for ids in ids.chunks(200) {
            allowed.extend(target.phrase_ids(a.id(), ids, phrases).await?);
        }
        matches.retain(|m| allowed.contains(&m.message.id));
        Ok(matches)
    }
    /// Revalidate frozen result identities against the live application store.
    /// This works when search storage is separate or temporarily unavailable.
    pub async fn current_indexed_search_rows(
        &self,
        a: &crate::AuthorizedAccount,
        ids: &[String],
        filter: &CachedSearchFilter<'_>,
    ) -> Result<Vec<Message>> {
        let _guard = self.gate.read().await;
        let repository = self.account_repository(a).await?;
        if ids.len() > 200 {
            return Err(Error::Invalid);
        }
        let excluded = self.search_excluded_folders(a).await?;
        Ok(repository
            .current_search_messages(ids)
            .await?
            .into_iter()
            .filter_map(|(message, snoozed)| {
                if !filter.classification.matches(&message, &crate::now())
                    || !filter.matches_date(&message)
                    || message.folder_ids.is_empty()
                    || message
                        .folder_ids
                        .iter()
                        .any(|f| excluded.iter().any(|e| e.eq_ignore_ascii_case(f)))
                    || filter.seen.is_some_and(|s| message.unread == s)
                    || filter.starred.is_some_and(|s| message.starred != s)
                    || filter
                        .folders
                        .is_some_and(|fs| !message.folder_ids.iter().any(|f| fs.contains(f)))
                    || filter
                        .category
                        .is_some_and(|c| message.category.as_ref().is_some_and(|v| v != c))
                    || (filter.snoozed_only && !snoozed)
                    || (filter.hide_snoozed && snoozed)
                {
                    None
                } else {
                    Some(message)
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod query_plan_tests {
    use super::*;
    #[tokio::test]
    async fn bounded_semantic_candidates_use_integer_primary_keys() {
        let dir = tempfile::tempdir().unwrap();
        let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap()
        else {
            unreachable!()
        };
        let plan: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(&format!("EXPLAIN QUERY PLAN {SQLITE_PAYLOADS}"))
                .bind("[1,2,3]")
                .bind("account")
                .bind("revision")
                .fetch_all(&db)
                .await
                .unwrap();
        assert!(
            plan.iter()
                .any(|row| row.3.contains("SEARCH c USING INTEGER PRIMARY KEY")),
            "{plan:?}"
        );
        assert!(
            plan.iter().any(|row| row.3.contains(
                "SEARCH d USING COVERING INDEX documents_filter_cohort (account_id=? AND rowid=?)"
            )),
            "{plan:?}"
        );
        assert!(
            plan.iter().any(|row| row
                .3
                .contains("SEARCH identity USING COVERING INDEX sqlite_autoindex_documents_1")),
            "{plan:?}"
        );
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::attachment_excerpt;
    #[test]
    fn malformed_or_legacy_ranges_never_invent_a_file_or_slice_utf8() {
        for details in [
            "not json",
            r#"[{"filename":"legacy.pdf","state":"ready"}]"#,
            r#"[{"filename":"bad.pdf","byte_start":1,"byte_end":2}]"#,
            r#"[{"filename":"bad.pdf","byte_start":-10,"byte_end":999}]"#,
        ] {
            let text = "évidence".to_owned();
            let (excerpt, file) = attachment_excerpt(text.clone(), 2, details, 0, 9);
            assert_eq!(file, None);
            assert_eq!(excerpt, text);
        }
        let (text, file) = attachment_excerpt(
            "body".into(),
            1,
            r#"[{"filename":"wrong","byte_start":0,"byte_end":4}]"#,
            0,
            4,
        );
        assert_eq!(text, "body");
        assert_eq!(file, None);
    }
}
