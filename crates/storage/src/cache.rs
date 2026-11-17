use super::*;

pub(crate) const STARRED_PREDICATE: &str = "json_extract(document,'$.starred')=1 AND json_array_length(document,'$.folder_ids')>0
             AND CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END";
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
        provider: &str,
    ) -> Result<Option<Message>> {
        let mut timing = timing::Timing::new("identity_pool_acquire");
        let repository = self.account_repository(a).await?;
        timing.stage("identity_query");
        let result = repository.known_provider_message(provider).await;
        timing.stage("identity_decode");
        result
    }
    /// Saved starred navigation remains usable without a mail provider. This is
    /// a local projection, never a promise of complete provider coverage.
    pub async fn starred_rows(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
    ) -> Result<Vec<Message>> {
        self.starred_rows_filtered(a, before, category, &ClassificationFilter::default())
            .await
    }
    pub async fn starred_rows_filtered(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
        filter: &ClassificationFilter,
    ) -> Result<Vec<Message>> {
        self.account_repository(a)
            .await?
            .starred_rows(before, category, filter)
            .await
    }
    /// Bounded keyset walk, including already cached rows so missing/legacy
    /// blobs are repaired. Never use an obsolete IMAP UIDVALIDITY identity.
    pub async fn starred_cache_rows(
        &self,
        a: &AuthorizedAccount,
        after: &str,
    ) -> Result<Vec<Message>> {
        self.account_repository(a)
            .await?
            .starred_cache_rows(after)
            .await
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
        providers: &[String],
    ) -> Result<Vec<String>> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .message_ids(providers)
            .await
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
        let repository = self.account_repository(a).await?;
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
        let evicted = repository
            .put_cache(&content.message.id, &blob_id, bytes.len() as u64, budget)
            .await?;
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
        blobs: Vec<String>,
    ) -> Result<()> {
        let retained = blobs.into_iter().collect::<std::collections::HashSet<_>>();
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
        let repository = self.account_repository(a).await?;
        let blob = repository.cache_blob(id).await?;
        let Some(blob) = blob else { return Ok(None) };
        Uuid::parse_str(&blob).map_err(|_| Error::Invalid)?;
        let bytes = match tokio::fs::read(self.root.join("cache").join(a.id()).join(blob)).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        repository.touch_cache(id).await?;
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
        let repository = self.account_repository(a).await?;
        repository.evict_received().await?;
        let directory = self.root.join("cache").join(&a.id);
        if directory.exists() {
            tokio::fs::remove_dir_all(directory).await?;
        }
        Ok(())
    }
}
