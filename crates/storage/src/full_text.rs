//! Durable account-scoped search. Trigrams accelerate literal substrings, not a
//! new token grammar. All FTS candidates are checked against normalized text.
use super::*;
use unicode_normalization::UnicodeNormalization;

pub fn normalize_search(value: &str) -> String {
    if value.is_ascii() {
        return value.to_ascii_lowercase();
    }
    value
        .nfc()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .nfc()
        .collect()
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextCoverage {
    pub headers: i64,
    pub bodies: i64,
    pub pending: i64,
    pub gaps: i64,
    pub bytes: i64,
    pub dirty: i64,
    pub retrying: i64,
    pub delivery_pending: i64,
    pub eligible: i64,
    pub processed: i64,
    #[serde(default)]
    pub semantic_required: bool,
    #[serde(default)]
    pub lexical_pending: i64,
    #[serde(default)]
    pub lexical_failed: bool,
    #[serde(default)]
    pub lexical_discovered: bool,
    #[serde(default)]
    pub semantic_revision: Option<String>,
    #[serde(default)]
    pub semantic_available: bool,
    #[serde(default)]
    pub semantic_failed: bool,
    #[serde(default)]
    pub semantic_pending: i64,
    #[serde(default)]
    pub semantic_retrying: i64,
    #[serde(default)]
    pub backlog: i64,
}
use serde::{Deserialize, Serialize};

/// One source-guarded extraction commit. Raw attachment bytes are never retained.
pub struct SearchContent<'a> {
    pub message: &'a Message,
    pub text: &'a str,
    pub state: &'a str,
    pub participants: &'a str,
    pub recipients: &'a str,
    pub attachments: &'a str,
    pub attachment_details: &'a str,
}

impl Store {
    /// Bounded, resumable metadata migration and incremental header updates.
    pub async fn index_search_headers(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .index_search_headers()
            .await
    }
    async fn source_coverage(
        &self,
        a: &AuthorizedAccount,
    ) -> Result<(TextCoverage, Option<String>)> {
        let repository = self.account_repository(a).await?;
        let (mut coverage, identity) = repository
            .source_coverage(
                self.expected_search_model.as_deref(),
                self.search_embeddings_enabled(),
            )
            .await?;
        if self.search_database.is_none() {
            coverage.processed = coverage.bodies + coverage.gaps - coverage.retrying;
            coverage.delivery_pending = 0;
            coverage.lexical_discovered = true;
            coverage.semantic_required = false;
            coverage.semantic_revision = None;
            coverage.semantic_available = false;
            coverage.semantic_failed = false;
            coverage.semantic_pending = 0;
            coverage.backlog = coverage.dirty + coverage.pending + coverage.retrying;
        }
        Ok((coverage, identity))
    }

    /// Application-side discovery and delivery inventory remains available when
    /// the independently selected search database is unavailable.
    pub async fn indexing_inventory(&self, a: &AuthorizedAccount) -> Result<TextCoverage> {
        self.source_coverage(a).await.map(|(coverage, _)| coverage)
    }
    pub async fn text_coverage(&self, a: &AuthorizedAccount) -> Result<TextCoverage> {
        let (mut coverage, identity) = self.source_coverage(a).await?;
        if let Some(target) = &self.search_database {
            if identity.as_deref() != Some(target.identity().await?.as_str()) {
                if coverage.semantic_required {
                    coverage.backlog += coverage.processed;
                }
                coverage.processed = 0;
                if coverage.semantic_required {
                    coverage.semantic_pending = coverage.eligible;
                }
                coverage.delivery_pending += 1;
            }
            let lexical = target.lexical_coverage(a.id()).await?;
            coverage.lexical_pending = lexical.pending;
            coverage.lexical_failed = lexical.failed
                || lexical
                    .revision
                    .is_some_and(|r| r != crate::lexical_text::REVISION);
            if coverage.lexical_failed && lexical.revision != Some(crate::lexical_text::REVISION) {
                coverage.processed = 0;
                coverage.backlog = coverage.backlog.max(coverage.eligible);
            }
            coverage.lexical_discovered = lexical.discovered;
            if !coverage.semantic_required {
                let remaining = lexical.pending.min(coverage.eligible).max(0);
                coverage.processed = coverage.processed.min(coverage.eligible - remaining);
                coverage.backlog = coverage.backlog.max(remaining);
            }
            let stored = target.coverage(a.id()).await?;
            // Inventory reflects current eligibility immediately, even before a
            // tombstone reaches the destination. Physical indexed bytes belong
            // to the selected search store and may lag that removal briefly.
            coverage.bytes = stored.bytes;
            if coverage.semantic_required {
                match target.embedding_coverage(a.id()).await {
                    Ok(semantic) => {
                        coverage.semantic_available &= semantic.model_available;
                        coverage.semantic_retrying = semantic.retrying;
                        coverage.semantic_failed |=
                            semantic.model_revision.is_some() && !semantic.model_available;
                        if semantic.model_revision != coverage.semantic_revision {
                            coverage.backlog += coverage.processed;
                            coverage.processed = 0;
                            coverage.semantic_pending = coverage.eligible;
                            coverage.semantic_available = false;
                            coverage.semantic_failed |= semantic.model_revision.is_some();
                        }
                    }
                    Err(_) => {
                        coverage.semantic_available = false;
                        coverage.semantic_failed = true;
                    }
                }
            }
        }
        Ok(coverage)
    }
    /// Prioritize recent arrivals, but periodically service the oldest pending
    /// identity. Only transient fetch failures retry after an hour. Permanent
    /// gaps require changed source identity (or an explicit parser/limit upgrade).
    pub async fn next_search_body(
        &self,
        a: &AuthorizedAccount,
        oldest: bool,
    ) -> Result<Option<Message>> {
        Ok(self
            .next_search_bodies(a, oldest, 1)
            .await?
            .into_iter()
            .next())
    }
    /// Select one bounded page once, avoiding repeated scans over already
    /// indexed rows while filling the durable body index.
    pub async fn next_search_bodies(
        &self,
        a: &AuthorizedAccount,
        oldest: bool,
        limit: usize,
    ) -> Result<Vec<Message>> {
        self.account_repository(a)
            .await?
            .next_search_bodies(oldest, limit)
            .await
    }

    /// Commit only against the still-current identity and membership. Never
    /// resurrect text fetched concurrently with confirmed deletion/epoch reset.
    pub async fn put_search_body(
        &self,
        a: &AuthorizedAccount,
        m: &Message,
        text: &str,
        state: &str,
    ) -> Result<()> {
        self.put_search_bodies(a, &[(m, text, state)]).await
    }
    /// One atomic bounded body-index commit, retaining per-message identity,
    /// membership checks inside the writer transaction.
    pub async fn put_search_bodies(
        &self,
        a: &AuthorizedAccount,
        rows: &[(&Message, &str, &str)],
    ) -> Result<()> {
        let records: Vec<_> = rows
            .iter()
            .map(|(message, text, state)| SearchContent {
                message,
                text,
                state,
                participants: "",
                recipients: "",
                attachments: "",
                attachment_details: "[]",
            })
            .collect();
        self.put_search_records(a, &records).await
    }
    pub async fn put_search_records(
        &self,
        a: &AuthorizedAccount,
        records: &[SearchContent<'_>],
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .put_search_records(records)
            .await
    }

    /// Durable scan coverage is independent of wall-clock age and body success.
    /// Callers supply the known selectable folders. Resets, unfinished scans,
    /// limited coverage and changed provider epochs invalidate completion.
    pub async fn search_scans_complete(
        &self,
        a: &AuthorizedAccount,
        folders: &[String],
    ) -> Result<bool> {
        self.account_repository(a)
            .await?
            .search_scans_complete(folders)
            .await
    }

    /// Keyset page over current metadata, with no attachment or MIME retention.
    /// Dates deliberately remain provider-only: UTC is not INTERNALDATE-day.
    pub async fn full_text_rows(
        &self,
        a: &AuthorizedAccount,
        filter: &crate::CachedSearchFilter<'_>,
        before: Option<(&str, &str)>,
    ) -> Result<Vec<Message>> {
        if self.search_database.is_some() {
            return self.independent_search_rows(a, filter, before).await;
        }
        let db = self.db(a).await?;
        let from = filter.from.map(normalize_search);
        let to = filter.to.map(normalize_search);
        let subject = filter.subject.map(normalize_search);
        let text = filter.text.map(normalize_search);
        let mut fts = Vec::new();
        for (column, value) in [
            ("sender", &from),
            ("{recipients mime_recipients}", &to),
            ("subject", &subject),
            (
                "{sender recipients subject body participants mime_recipients attachment_text}",
                &text,
            ),
        ] {
            if let Some(value) = value
                .as_ref()
                .filter(|v| v.chars().count() >= 3 && !v.contains('\0'))
            {
                fts.push(format!("{column}:\"{}\"", value.replace('"', "\"\"")));
            }
        }
        let expression = fts.join(" AND ");
        let sparse = if expression.is_empty() {
            false
        } else {
            let hits:i64=sqlx::query_scalar("SELECT COUNT(*) FROM (SELECT rowid FROM search_fts WHERE search_fts MATCH ? LIMIT 512)").bind(&expression).fetch_one(&db).await?;
            hits < 512
        };
        let source = if sparse {
            "search_documents d"
        } else {
            "search_documents d INDEXED BY search_order"
        };
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!("SELECT m.document FROM {source} JOIN messages m ON m.id=d.message_id WHERE EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.id=m.id) AND json_array_length(m.document,'$.folder_ids')>0 AND d.source=json_extract(m.document,'$.provider_id') AND NOT EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) AND NOT EXISTS(SELECT 1 FROM search_dirty WHERE message_id=m.id) "));
        if !expression.is_empty() {
            q.push(" AND d.rowid IN (SELECT rowid FROM search_fts WHERE search_fts MATCH ")
                .push_bind(&expression)
                .push(")");
        }
        for (column, value) in [
            ("sender", &from),
            ("recipients", &to),
            ("subject", &subject),
        ] {
            if let Some(value) = value {
                if column == "recipients" {
                    q.push(" AND (instr(d.recipients,")
                        .push_bind(value)
                        .push(")>0 OR instr(d.mime_recipients,")
                        .push_bind(value)
                        .push(")>0)");
                    continue;
                }
                q.push(format!(" AND instr(d.{column},"))
                    .push_bind(value)
                    .push(")>0");
            }
        }
        if let Some(value) = &text {
            q.push(" AND (instr(d.sender,")
                .push_bind(value)
                .push(")>0 OR instr(d.recipients,")
                .push_bind(value)
                .push(")>0 OR instr(d.subject,")
                .push_bind(value)
                .push(")>0 OR instr(d.body,")
                .push_bind(value)
                .push(")>0 OR instr(d.participants,")
                .push_bind(value)
                .push(")>0 OR instr(d.mime_recipients,")
                .push_bind(value)
                .push(")>0 OR instr(d.attachment_text,")
                .push_bind(value)
                .push(")>0)");
        }
        for (operator, day) in [(">=", filter.since), ("<", filter.before)] {
            if let Some(day) = day {
                q.push(" AND json_extract(m.document,'$.received_date')")
                    .push(operator)
                    .push_bind(day);
            }
        }
        if let Some(value) = filter.seen {
            q.push(" AND json_extract(m.document,'$.unread')<>")
                .push_bind(value);
        }
        if let Some(value) = filter.starred {
            q.push(" AND json_extract(m.document,'$.starred')=")
                .push_bind(value);
        }
        let folders = filter.folders.map(serde_json::to_string).transpose()?;
        if let Some(value) = &folders {
            q.push(" AND EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE f.value IN (SELECT value FROM json_each(").push_bind(value).push(")))");
        }
        crate::classification::predicate!(q, true, "m.document", &filter.classification);
        if let Some(value) = filter.category {
            q.push(" AND (json_extract(m.document,'$.category') IS NULL OR json_extract(m.document,'$.category')=").push_bind(value).push(")");
        }
        if filter.snoozed_only || filter.hide_snoozed {
            q.push(if filter.hide_snoozed { " AND NOT EXISTS(" } else { " AND EXISTS(" }).push("SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>").push_bind(now()).push(")");
        }
        if let Some((date, id)) = before {
            q.push(" AND (d.received_at,d.message_id)<(")
                .push_bind(date)
                .push(",")
                .push_bind(id)
                .push(")");
        }
        q.push(" ORDER BY d.received_at DESC,d.message_id DESC LIMIT 51");
        let rows: Vec<(String,)> = q.build_query_as().fetch_all(&db).await?;
        rows.into_iter()
            .map(|(raw,)| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
}
