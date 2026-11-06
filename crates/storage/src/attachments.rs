use super::*;
impl Store {
    pub async fn put_attachment(
        &self,
        a: &AuthorizedAccount,
        filename: String,
        content_type: String,
        bytes: &[u8],
    ) -> Result<Attachment> {
        self.pin_attachment(
            a,
            &Uuid::new_v4().to_string(),
            filename,
            content_type,
            bytes,
        )
        .await
    }
    /// Preserve the account-scoped received part ID when deliberately forwarding.
    pub async fn pin_attachment(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        filename: String,
        content_type: String,
        bytes: &[u8],
    ) -> Result<Attachment> {
        Uuid::parse_str(id).map_err(|_| Error::Invalid)?;
        // Serialize quota reservation, durable write and orphan collection.
        let _guard = self.gate.write().await;
        if let Ok((metadata, _)) = self.attachment(a, id).await {
            return Ok(metadata);
        }
        if bytes.len() > 20 * 1024 * 1024
            || filename.is_empty()
            || filename.len() > 255
            || filename.contains(['\r', '\n', '\0'])
            || content_type.len() > 200
            || content_type.contains(['\r', '\n', '\0'])
        {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let attachment = Attachment {
            id: id.to_owned(),
            filename,
            content_type,
            size_bytes: bytes.len() as u32,
        };
        let directory = self.root.join("durable").join(&a.id);
        tokio::fs::create_dir_all(&directory).await?;
        // Count actual bytes, including interrupted writes not yet indexed in SQLite.
        let mut used = 0_u64;
        let mut entries = tokio::fs::read_dir(&directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let metadata = tokio::fs::symlink_metadata(entry.path()).await?;
            if metadata.is_file() {
                used = used.saturating_add(metadata.len());
            }
        }
        if used.saturating_add(bytes.len() as u64) > self.attachment_budget {
            return Err(Error::QuotaExceeded);
        }
        let path = directory.join(&attachment.id);
        // New UUID and exclusive create keep browser-provided names out of paths.
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        let mut tx = db.begin().await?;
        sqlx::query("INSERT INTO blobs(id,size_bytes,pinned,last_access) VALUES(?,?,1,?)")
            .bind(&attachment.id)
            .bind(attachment.size_bytes)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO attachment_metadata(id,document) VALUES(?,?)")
            .bind(&attachment.id)
            .bind(serde_json::to_string(&attachment)?)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(attachment)
    }
    /// Collect abandoned uploads only after a day. Every immutable submission,
    /// including uncertain/accepted records, keeps its snapshot attachments.
    pub async fn cleanup_orphan_attachments(&self, a: &AuthorizedAccount) -> Result<u32> {
        let _guard = self.gate.write().await;
        let db = self.db(a).await?;
        let cutoff = (Utc::now() - Duration::days(1)).to_rfc3339_opts(SecondsFormat::Millis, true);
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE blobs SET pinned=pinned WHERE 0")
            .execute(&mut *tx)
            .await?;
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM blobs WHERE last_access<? AND NOT EXISTS(SELECT 1 FROM draft_blobs WHERE blob_id=blobs.id) AND NOT EXISTS(SELECT 1 FROM submissions,json_each(submissions.snapshot,'$.attachments') part WHERE json_extract(part.value,'$.id')=blobs.id) LIMIT 100")
            .bind(cutoff).fetch_all(&mut *tx).await?;
        for id in &ids {
            Uuid::parse_str(id).map_err(|_| Error::Invalid)?;
            sqlx::query("DELETE FROM attachment_metadata WHERE id=?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM blobs WHERE id=?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        for id in &ids {
            match tokio::fs::remove_file(self.root.join("durable").join(&a.id).join(id)).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        // A crash between create/write and the DB commit can leave an unindexed
        // file. Only old UUID-named regular files with no durable references qualify.
        let directory = self.root.join("durable").join(&a.id);
        let mut removed = ids.len() as u32;
        let mut entries = match tokio::fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(removed),
            Err(error) => return Err(error.into()),
        };
        while removed < 100 {
            let Some(entry) = entries.next_entry().await? else {
                break;
            };
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if Uuid::parse_str(&id).is_err() {
                continue;
            }
            let metadata = tokio::fs::symlink_metadata(entry.path()).await?;
            if !metadata.is_file()
                || metadata.modified()?.elapsed().unwrap_or_default().as_secs() < 86400
            {
                continue;
            }
            let retained: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM blobs WHERE id=?) OR EXISTS(SELECT 1 FROM attachment_metadata WHERE id=?) OR EXISTS(SELECT 1 FROM draft_blobs WHERE blob_id=?) OR EXISTS(SELECT 1 FROM submissions,json_each(submissions.snapshot,'$.attachments') part WHERE json_extract(part.value,'$.id')=?)")
                .bind(&id).bind(&id).bind(&id).bind(&id).fetch_one(&db).await?;
            if !retained {
                tokio::fs::remove_file(entry.path()).await?;
                removed += 1;
            }
        }
        Ok(removed)
    }
    pub async fn attachment(
        &self,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<(Attachment, Vec<u8>)> {
        let db = self.db(a).await?;
        Uuid::parse_str(id).map_err(|_| Error::NotFound)?;
        let raw: Option<String> =
            sqlx::query_scalar("SELECT document FROM attachment_metadata WHERE id=?")
                .bind(id)
                .fetch_optional(&db)
                .await?;
        let metadata = serde_json::from_str(&raw.ok_or(Error::NotFound)?)?;
        let bytes = tokio::fs::read(self.root.join("durable").join(&a.id).join(id)).await?;
        Ok((metadata, bytes))
    }
}
