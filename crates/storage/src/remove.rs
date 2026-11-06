use super::*;
impl Store {
    /// Revoke local access and credentials before removing local files. Never calls a provider.
    pub async fn remove_account(&self, a: &AuthorizedAccount, confirmation: &str) -> Result<()> {
        let _guard = self.gate.write().await;
        let db = self.db(a).await?;
        let metadata: String =
            sqlx::query_scalar("SELECT metadata FROM accounts WHERE id=? AND user_id=?")
                .bind(a.id())
                .bind(a.user_id())
                .fetch_one(&self.control)
                .await?;
        let account: Account = serde_json::from_str(&metadata)?;
        if confirmation != account.email {
            return Err(Error::Invalid);
        }
        // Never discard uncertain delivery evidence, scheduled sends, or in-flight mail actions.
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM submissions WHERE state IN ('scheduled','claimed','submitting','delivery_unknown') OR (state='accepted' AND filing_status IN ('pending','failed'))) OR EXISTS(SELECT 1 FROM outbox WHERE kind='mutation' AND state IN ('pending','running'))")
            .fetch_one(&db).await?;
        if busy {
            return Err(Error::AccountBusy);
        }
        // No concurrent reader can reopen between revocation and cleanup.
        let mut pools = self.account_pools.lock().await;
        let mut tx = self.control.begin().await?;
        sqlx::query("INSERT INTO removed_accounts(id) VALUES(?)")
            .bind(a.id())
            .execute(&mut *tx)
            .await?;
        let count = sqlx::query("DELETE FROM accounts WHERE id=? AND user_id=?")
            .bind(a.id())
            .bind(a.user_id())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if count != 1 {
            return Err(Error::NotFound);
        }
        // Invalidate saved search results that might contain removed account metadata.
        sqlx::query("DELETE FROM search_cursors WHERE user_id=?")
            .bind(a.user_id())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        pools.remove(a.id());
        db.close().await;
        // Revocation is already committed. Interrupted filesystem cleanup is retried on startup.
        let _ = self.cleanup_removed_accounts().await;
        Ok(())
    }
    pub(crate) async fn cleanup_removed_accounts(&self) -> Result<()> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM removed_accounts")
            .fetch_all(&self.control)
            .await?;
        for id in ids {
            Uuid::parse_str(&id).map_err(|_| Error::Invalid)?;
            for suffix in [".sqlite", ".sqlite-wal", ".sqlite-shm"] {
                match tokio::fs::remove_file(
                    self.root.join("accounts").join(format!("{id}{suffix}")),
                )
                .await
                {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            for directory in ["durable", "cache"] {
                match tokio::fs::remove_dir_all(self.root.join(directory).join(&id)).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(())
    }
}
