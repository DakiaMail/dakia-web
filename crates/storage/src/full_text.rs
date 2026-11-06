//! Durable account-scoped search. Trigrams accelerate literal substrings, not a
//! new token grammar. All FTS candidates are checked against normalized text.
use super::*;
use unicode_normalization::UnicodeNormalization;

pub fn normalize_search(value: &str) -> String {
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
}
use serde::{Deserialize, Serialize};
const SEARCH_BUDGET: i64 = 512 * 1024 * 1024;

impl Store {
    /// Bounded, resumable metadata migration and incremental header updates.
    pub async fn index_search_headers(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let rows: Vec<String> = sqlx::query_scalar("SELECT m.document FROM search_dirty d JOIN messages m ON m.id=d.message_id ORDER BY d.message_id LIMIT 200").fetch_all(&mut *tx).await?;
        for raw in rows {
            let m: Message = serde_json::from_str(&raw)?;
            let live: bool = sqlx::query_scalar("SELECT CASE WHEN json_valid(?) THEN NOT EXISTS(SELECT 1 FROM provider_documents WHERE key='thread:epoch:'||json_extract(?,'$.folder') AND CAST(document AS INTEGER)<>json_extract(?,'$.uidvalidity')) ELSE 1 END")
                .bind(&m.provider_id).bind(&m.provider_id).bind(&m.provider_id).fetch_one(&mut *tx).await?;
            if !m.folder_ids.is_empty() && live {
                let address =
                    |v: &Address| format!("{} {}", v.email, v.name.as_deref().unwrap_or(""));
                let to = m.to.iter().map(address).collect::<Vec<_>>().join("\n");
                sqlx::query("INSERT INTO search_documents(message_id,source,sender,recipients,subject,received_at) VALUES(?,?,?,?,?,?) ON CONFLICT(message_id) DO UPDATE SET source=excluded.source,sender=excluded.sender,recipients=excluded.recipients,subject=excluded.subject,received_at=excluded.received_at,body=CASE WHEN source=excluded.source THEN body ELSE '' END,body_state=CASE WHEN source=excluded.source THEN body_state ELSE 'pending' END,retry_at=CASE WHEN source=excluded.source THEN retry_at ELSE 0 END WHERE source<>excluded.source OR sender<>excluded.sender OR recipients<>excluded.recipients OR subject<>excluded.subject OR received_at<>excluded.received_at")
                    .bind(&m.id).bind(&m.provider_id).bind(normalize_search(&address(&m.sender))).bind(normalize_search(&to)).bind(normalize_search(&m.subject)).bind(&m.received_at).execute(&mut *tx).await?;
            } else {
                sqlx::query("DELETE FROM search_documents WHERE message_id=?")
                    .bind(&m.id)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("DELETE FROM search_dirty WHERE message_id=?")
                .bind(&m.id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn text_coverage(&self, a: &AuthorizedAccount) -> Result<TextCoverage> {
        let db = self.db(a).await?;
        let row = sqlx::query("SELECT COUNT(*) headers,COALESCE(SUM(body_state='ready'),0) bodies,COALESCE(SUM(body_state='pending'),0) pending,COALESCE(SUM(body_state NOT IN ('ready','pending')),0) gaps,COALESCE(SUM(length(CAST(body AS BLOB))),0) bytes,(SELECT COUNT(*) FROM search_dirty) dirty FROM search_documents").fetch_one(&db).await?;
        Ok(TextCoverage {
            headers: row.get("headers"),
            bodies: row.get("bodies"),
            pending: row.get("pending"),
            gaps: row.get("gaps"),
            bytes: row.get("bytes"),
            dirty: row.get("dirty"),
        })
    }
    /// Prioritize recent arrivals, but periodically service the oldest pending
    /// identity. Only transient fetch failures retry after an hour. Permanent
    /// gaps require changed source identity (or an explicit parser/limit upgrade).
    /// Quota gaps retry only after room exists for the maximum permitted text.
    pub async fn next_search_body(
        &self,
        a: &AuthorizedAccount,
        oldest: bool,
    ) -> Result<Option<Message>> {
        let db = self.db(a).await?;
        let order = if oldest {
            "d.message_id"
        } else {
            "json_extract(m.document,'$.received_at') DESC,d.message_id"
        };
        let raw: Option<String> = sqlx::query_scalar(&format!("SELECT m.document FROM search_documents d JOIN messages m ON m.id=d.message_id WHERE d.body_state IN ('pending','fetch_error','quota') AND (d.body_state<>'quota' OR (SELECT body_bytes FROM search_totals WHERE id=1)<=?) AND d.retry_at<=? AND json_array_length(m.document,'$.folder_ids')>0 ORDER BY {order} LIMIT 1"))
            .bind(SEARCH_BUDGET - 1024 * 1024).bind(Utc::now().timestamp()).fetch_optional(&db).await?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(Error::from))
            .transpose()
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
        if m.account_id != a.id
            || !matches!(
                state,
                "ready" | "encrypted" | "oversized" | "parse_error" | "fetch_error" | "quota"
            )
        {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let text = normalize_search(text);
        let bytes: i64 = sqlx::query_scalar("SELECT body_bytes-COALESCE((SELECT length(CAST(body AS BLOB)) FROM search_documents WHERE message_id=?),0) FROM search_totals WHERE id=1").bind(&m.id).fetch_one(&mut *tx).await?;
        let state = if text.len() > 1024 * 1024 {
            "oversized"
        } else if bytes + text.len() as i64 > SEARCH_BUDGET {
            "quota"
        } else {
            state
        };
        sqlx::query("UPDATE search_documents SET body=?,body_state=?,retry_at=?,updated_at=? WHERE message_id=? AND source=? AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_array_length(document,'$.folder_ids')>0 AND json_extract(document,'$.provider_id')=?)")
            .bind(if state=="ready" { text.as_str() } else { "" }).bind(state).bind(if state=="fetch_error" { Utc::now().timestamp()+3600 } else { 0 }).bind(Utc::now().timestamp()).bind(&m.id).bind(&m.provider_id).bind(&m.id).bind(&m.provider_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Freshness is independent of pagination and body success. Callers must
    /// supply the complete, recently discovered selectable folder list.
    pub async fn search_scans_fresh(
        &self,
        a: &AuthorizedAccount,
        folders: &[String],
    ) -> Result<bool> {
        let db = self.db(a).await?;
        if folders.is_empty() {
            return Ok(false);
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folder_scans WHERE folder IN (SELECT value FROM json_each(?)) AND completed_at IS NOT NULL AND coverage_limited=0 AND continuation IS NULL AND last_scanned_at>?")
            .bind(serde_json::to_string(folders)?).bind(Utc::now().timestamp()-300).fetch_one(&db).await?;
        Ok(count == folders.len() as i64)
    }
    /// Keyset page over current metadata, with no attachment or MIME retention.
    /// Dates deliberately remain provider-only: UTC is not INTERNALDATE-day.
    pub async fn full_text_rows(
        &self,
        a: &AuthorizedAccount,
        filter: &crate::CachedSearchFilter<'_>,
        before: Option<(&str, &str)>,
    ) -> Result<Vec<Message>> {
        let db = self.db(a).await?;
        let from = filter.from.map(normalize_search);
        let to = filter.to.map(normalize_search);
        let subject = filter.subject.map(normalize_search);
        let text = filter.text.map(normalize_search);
        let mut fts = Vec::new();
        for (column, value) in [
            ("sender", &from),
            ("recipients", &to),
            ("subject", &subject),
            ("{sender recipients subject body}", &text),
        ] {
            if let Some(value) = value
                .as_ref()
                .filter(|v| v.chars().count() >= 3 && !v.contains('\0'))
            {
                fts.push(format!("{column}:\"{}\"", value.replace('"', "\"\"")));
            }
        }
        let expression = fts.join(" AND ");
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT m.document FROM search_documents d INDEXED BY search_order JOIN messages m ON m.id=d.message_id WHERE json_array_length(m.document,'$.folder_ids')>0 AND d.source=json_extract(m.document,'$.provider_id') AND NOT EXISTS(SELECT 1 FROM search_dirty WHERE message_id=m.id) ");
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
                .push(")>0)");
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
