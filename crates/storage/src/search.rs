use super::*;
#[derive(Default)]
pub struct CachedSearchFilter<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub text: Option<&'a str>,
    pub seen: Option<bool>,
    pub starred: Option<bool>,
    pub folders: Option<&'a [String]>,
    pub classification: ClassificationFilter,
    pub category: Option<&'a str>,
    pub snoozed_only: bool,
    pub hide_snoozed: bool,
}
impl Store {
    /// Search all cached metadata, returning a bounded positive preview, never
    /// evidence of absence. No bodies, snippets or provider calls are involved.
    pub async fn cached_search_rows(
        &self,
        a: &AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
    ) -> Result<Vec<Message>> {
        let db = self.db(a).await?;
        let folders = filter.folders.map(serde_json::to_string).transpose()?;
        let rows: Vec<String> = sqlx::query_scalar(&format!(
            "SELECT m.document FROM messages m CROSS JOIN (SELECT ? AS filter) cf
             WHERE CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             AND (? IS NULL OR instr(lower(json_extract(m.document,'$.subject')),lower(?))>0)
             AND (? IS NULL OR instr(lower(json_extract(m.document,'$.sender.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(m.document,'$.sender.name'),'')),lower(?))>0)
             AND (? IS NULL OR EXISTS(SELECT 1 FROM json_each(m.document,'$.to') t WHERE instr(lower(json_extract(t.value,'$.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(t.value,'$.name'),'')),lower(?))>0))
             AND (? IS NULL OR instr(lower(json_extract(m.document,'$.subject')),lower(?))>0 OR instr(lower(json_extract(m.document,'$.sender.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(m.document,'$.sender.name'),'')),lower(?))>0 OR EXISTS(SELECT 1 FROM json_each(m.document,'$.to') t WHERE instr(lower(json_extract(t.value,'$.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(t.value,'$.name'),'')),lower(?))>0))
             AND (? IS NULL OR json_extract(m.document,'$.unread')<>?)
             AND (? IS NULL OR json_extract(m.document,'$.starred')=?)
             AND (? IS NULL OR EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE f.value IN (SELECT value FROM json_each(?))))
             {CLASSIFICATION_SQL}
             AND (? IS NULL OR json_extract(m.document,'$.category') IS NULL OR json_extract(m.document,'$.category')=?)
             AND (?=0 OR EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?))
             AND (?=0 OR NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?))
             ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 50",
        ))
        .bind(serde_json::to_string(&filter.classification)?)
        .bind(filter.subject).bind(filter.subject)
        .bind(filter.from).bind(filter.from).bind(filter.from)
        .bind(filter.to).bind(filter.to).bind(filter.to)
        .bind(filter.text).bind(filter.text).bind(filter.text).bind(filter.text).bind(filter.text).bind(filter.text)
        .bind(filter.seen).bind(filter.seen).bind(filter.starred).bind(filter.starred)
        .bind(&folders).bind(&folders).bind(filter.category).bind(filter.category)
        .bind(filter.snoozed_only).bind(now()).bind(filter.hide_snoozed).bind(now())
        .fetch_all(&db).await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }

    /// Opaque, owner-bound continuation. Mailbox state never enters URLs.
    pub async fn save_search_cursor(&self, user: &str, document: &str) -> Result<String> {
        let _guard = self.gate.read().await;
        let mut tx = self.control.begin().await?;
        sqlx::query("DELETE FROM search_cursors WHERE expires_at<=?")
            .bind(Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;
        let id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO search_cursors(id,user_id,document,expires_at) VALUES(?,?,?,?)")
            .bind(&id)
            .bind(user)
            .bind(document)
            .bind(Utc::now().timestamp() + 3600)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn load_search_cursor(&self, user: &str, id: &str) -> Result<String> {
        Uuid::parse_str(id).map_err(|_| Error::CursorExpired)?;
        let raw: Option<String> = sqlx::query_scalar(
            "SELECT document FROM search_cursors WHERE id=? AND user_id=? AND expires_at>?",
        )
        .bind(id)
        .bind(user)
        .bind(Utc::now().timestamp())
        .fetch_optional(&self.control)
        .await?;
        raw.ok_or(Error::CursorExpired)
    }
}
