use super::*;
impl Store {
    /// Revoke local access and credentials before removing local files. Never calls a provider.
    pub async fn remove_account(&self, a: &AuthorizedAccount, confirmation: &str) -> Result<()> {
        let _guard = self.gate.write().await;
        let repository = self.account_repository(a).await?;
        let account = self
            .control_repository()
            .account(a.user_id(), a.id())
            .await?;
        if confirmation != account.email {
            return Err(Error::Invalid);
        }
        // Queued mailbox changes leave with the account; protect deliveries and running writes.
        let busy = repository.removal_busy().await?;
        if busy {
            return Err(Error::AccountBusy);
        }
        // No concurrent reader can reopen between revocation and cleanup.
        let mut pools = self.account_pools.lock().await;
        self.control_repository()
            .revoke_account(a.user_id(), a.id())
            .await?;
        pools.remove(a.id());
        repository.close_removed_account().await;
        // Revocation is already committed. Interrupted filesystem cleanup is retried on startup.
        self.search_cleanup_done
            .store(false, std::sync::atomic::Ordering::Release);
        if self.cleanup_removed_accounts().await.is_ok() {
            self.search_cleanup_done
                .store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(())
    }
    pub(crate) async fn cleanup_removed_accounts(&self) -> Result<()> {
        let ids = self.control_repository().removed_accounts().await?;
        for id in &ids {
            Uuid::parse_str(id).map_err(|_| Error::Invalid)?;
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
                match tokio::fs::remove_dir_all(self.root.join(directory).join(id)).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        // Local cleanup does not wait for a remote search outage. The durable
        // journal remains available to retry destination cleanup afterward.
        if let Some(search) = &self.search_database {
            for id in ids {
                search.remove_account(&id).await?;
            }
        }
        Ok(())
    }
}
