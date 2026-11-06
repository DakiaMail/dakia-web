use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};

// A failed/cancelled transaction never owns its newly created cache file.
// Startup reconciliation handles process death before this destructor can run.
struct UncommittedCacheFile(Option<PathBuf>);
impl Drop for UncommittedCacheFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl Store {
    /// Read-only indexed lookup. Allocating an identity is not proof that its
    /// headers were successfully ingested before a crash.
    pub async fn known_provider_message(
        &self,
        a: &AuthorizedAccount,
        provider_id: &str,
    ) -> Result<Option<Message>> {
        let db = self.db(a).await?;
        let mut timing = timing::Timing::new("identity_pool_acquire");
        let mut connection = db.acquire().await?;
        timing.stage("identity_query");
        let raw: Option<String> = sqlx::query_scalar("SELECT m.document FROM message_identities i JOIN messages m ON m.id=i.message_id WHERE i.provider_id=?")
            .bind(provider_id).fetch_optional(&mut *connection).await?;
        timing.stage("identity_decode");
        raw.map(|s| serde_json::from_str(&s).map_err(Error::from))
            .transpose()
    }
    /// Saved starred navigation remains usable without a mail provider. This is
    /// a local projection, never a promise of complete provider coverage.
    pub async fn starred_rows(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
    ) -> Result<Vec<Message>> {
        let db = self.db(a).await?;
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT document FROM messages m WHERE json_extract(document,'$.starred')=1 AND json_array_length(document,'$.folder_ids')>0
             AND CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             AND (? IS NULL OR json_extract(document,'$.category') IS NULL OR json_extract(document,'$.category')=?)
             AND (? IS NULL OR (json_extract(document,'$.received_at'),id)<(?,?))
             ORDER BY json_extract(document,'$.received_at') DESC,id DESC LIMIT 51",
        ).bind(category).bind(category)
            .bind(before.map(|b| b.0)).bind(before.map(|b| b.0)).bind(before.map(|b| b.1))
            .fetch_all(&db).await?;
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    /// Bounded keyset walk, including already cached rows so missing/legacy
    /// blobs are repaired. Never use an obsolete IMAP UIDVALIDITY identity.
    pub async fn starred_cache_rows(
        &self,
        a: &AuthorizedAccount,
        after: &str,
    ) -> Result<Vec<Message>> {
        let db = self.db(a).await?;
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT document FROM messages m WHERE id>? AND json_extract(document,'$.starred')=1 AND json_array_length(document,'$.folder_ids')>0
             AND CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             ORDER BY id LIMIT 2",
        )
        .bind(after)
        .fetch_all(&db)
        .await?;
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    pub async fn message_id(&self, a: &AuthorizedAccount, provider_id: &str) -> Result<String> {
        Ok(self
            .message_ids(a, &[provider_id.to_owned()])
            .await?
            .remove(0))
    }
    pub async fn message_ids(
        &self,
        a: &AuthorizedAccount,
        provider_ids: &[String],
    ) -> Result<Vec<String>> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let mut ids = Vec::with_capacity(provider_ids.len());
        for provider_id in provider_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO message_identities(provider_id,message_id) VALUES(?,?)",
            )
            .bind(provider_id)
            .bind(Uuid::new_v4().to_string())
            .execute(&mut *tx)
            .await?;
            ids.push(
                sqlx::query_scalar("SELECT message_id FROM message_identities WHERE provider_id=?")
                    .bind(provider_id)
                    .fetch_one(&mut *tx)
                    .await?,
            );
        }
        tx.commit().await?;
        Ok(ids)
    }
    pub async fn cache_content(
        &self,
        a: &AuthorizedAccount,
        content: &MessageContent,
        budget: u64,
    ) -> Result<()> {
        self.cache_content_with_raw(a, content, None, budget).await
    }
    /// Raw MIME and parsed content share one bounded, evictable cache entry.
    /// They are never durable draft attachments unless explicitly forwarded.
    pub async fn cache_content_with_raw(
        &self,
        a: &AuthorizedAccount,
        content: &MessageContent,
        raw: Option<&[u8]>,
        budget: u64,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        if content.message.account_id != a.id {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let directory = self.root.join("cache").join(&a.id);
        tokio::fs::create_dir_all(&directory).await?;
        let mut value = serde_json::to_value(content)?;
        if let Some(raw) = raw {
            value["raw_mime"] = serde_json::Value::String(STANDARD.encode(raw));
        }
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() as u64 > budget {
            return Ok(());
        } // An oversized body remains provider-fetchable, not truncated.
        let blob_id = Uuid::new_v4().to_string();
        let path = directory.join(&blob_id);
        let mut uncommitted = UncommittedCacheFile(Some(path.clone()));
        tokio::fs::write(&path, &bytes).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE received_cache SET last_access=last_access WHERE message_id=?")
            .bind(&content.message.id)
            .execute(&mut *tx)
            .await?;
        let mut evicted: Vec<String> =
            sqlx::query_scalar("SELECT blob_id FROM received_cache WHERE message_id=?")
                .bind(&content.message.id)
                .fetch_all(&mut *tx)
                .await?;
        sqlx::query("INSERT INTO received_cache(message_id,blob_id,size_bytes,last_access) VALUES(?,?,?,?) ON CONFLICT(message_id) DO UPDATE SET blob_id=excluded.blob_id,size_bytes=excluded.size_bytes,last_access=excluded.last_access").bind(&content.message.id).bind(&blob_id).bind(bytes.len() as i64).bind(now()).execute(&mut *tx).await?;
        let mut total: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes),0) FROM received_cache")
                .fetch_one(&mut *tx)
                .await?;
        let rows=sqlx::query("SELECT c.message_id,c.blob_id,c.size_bytes FROM received_cache c JOIN messages m ON m.id=c.message_id ORDER BY json_extract(m.document,'$.starred'),c.last_access,c.message_id").fetch_all(&mut *tx).await?;
        for row in rows {
            if total as u64 <= budget {
                break;
            }
            total -= row.get::<i64, _>("size_bytes");
            let id: String = row.get("message_id");
            evicted.push(row.get("blob_id"));
            sqlx::query("DELETE FROM received_cache WHERE message_id=?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        uncommitted.0 = None;
        for id in evicted {
            let path = directory.join(id);
            if let Err(e) = tokio::fs::remove_file(path).await {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(e.into());
                }
            }
        }
        Ok(())
    }
    /// Reclaim only unreferenced UUID cache files before publishing an account
    /// pool. No live cache writer can use that pool until this finishes.
    pub(crate) async fn cleanup_received_orphans(
        &self,
        a: &AuthorizedAccount,
        db: &SqlitePool,
    ) -> Result<()> {
        let retained: std::collections::HashSet<String> =
            sqlx::query_scalar("SELECT blob_id FROM received_cache")
                .fetch_all(db)
                .await?
                .into_iter()
                .collect();
        let mut entries = match tokio::fs::read_dir(self.root.join("cache").join(a.id())).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if Uuid::parse_str(&name).is_ok()
                && !retained.contains(&name)
                && entry.file_type().await?.is_file()
            {
                tokio::fs::remove_file(entry.path()).await?;
            }
        }
        Ok(())
    }
    async fn cached_value(
        &self,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<Option<serde_json::Value>> {
        let db = self.db(a).await?;
        let blob: Option<String> =
            sqlx::query_scalar("SELECT blob_id FROM received_cache WHERE message_id=?")
                .bind(id)
                .fetch_optional(&db)
                .await?;
        let Some(blob) = blob else { return Ok(None) };
        Uuid::parse_str(&blob).map_err(|_| Error::Invalid)?;
        let bytes = match tokio::fs::read(self.root.join("cache").join(a.id()).join(blob)).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        sqlx::query("UPDATE received_cache SET last_access=? WHERE message_id=?")
            .bind(now())
            .bind(id)
            .execute(&db)
            .await?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }
    pub async fn cached_raw(&self, a: &AuthorizedAccount, id: &str) -> Result<Option<Vec<u8>>> {
        let Some(value) = self.cached_value(a, id).await? else {
            return Ok(None);
        };
        value
            .get("raw_mime")
            .and_then(|v| v.as_str())
            .map(|v| STANDARD.decode(v).map_err(|_| Error::Invalid))
            .transpose()
    }
    pub async fn cached_content(
        &self,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<Option<MessageContent>> {
        let Some(value) = self.cached_value(a, id).await? else {
            return Ok(None);
        };
        // Old body caches did not retain Reply-To. Refetch only a message the
        // user opens, rather than silently replying to an incorrect address.
        if value.get("reply_to").is_none() {
            return Ok(None);
        }
        let mut c: MessageContent = serde_json::from_value(value)?;
        // Cache bodies must not revert newer catalogue flags or manual corrections.
        c.message = self.message(a, id).await?;
        Ok(Some(c))
    }
    pub async fn evict_received(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.write().await;
        let db = self.db(a).await?;
        sqlx::query("DELETE FROM received_cache")
            .execute(&db)
            .await?;
        let directory = self.root.join("cache").join(&a.id);
        if directory.exists() {
            tokio::fs::remove_dir_all(directory).await?;
        }
        Ok(())
    }
}
