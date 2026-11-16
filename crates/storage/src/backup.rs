use super::*;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    created_at: String,
    accounts: Vec<String>,
    jobs_disabled: bool,
}
async fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    let mut stack = vec![(source.to_owned(), target.to_owned())];
    while let Some((source, target)) = stack.pop() {
        tokio::fs::create_dir_all(&target).await?;
        let mut entries = tokio::fs::read_dir(source).await?;
        while let Some(e) = entries.next_entry().await? {
            let kind = e.file_type().await?;
            if kind.is_symlink() {
                return Err(Error::Invalid);
            }
            if kind.is_dir() {
                stack.push((e.path(), target.join(e.file_name())))
            } else if kind.is_file() {
                tokio::fs::copy(e.path(), target.join(e.file_name())).await?;
            } else {
                return Err(Error::Invalid);
            }
        }
    }
    Ok(())
}
impl Store {
    /// A shared write gate pauses account mutations, including worker transitions.
    /// Snapshot accepted/submitting states without inventing delivery outcomes.
    pub async fn backup(&self, destination: impl AsRef<Path>) -> Result<()> {
        let _guard = self.gate.write().await;
        let destination = destination.as_ref();
        if destination.exists() {
            return Err(Error::Invalid);
        }
        tokio::fs::create_dir_all(destination.join("accounts")).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [destination.to_path_buf(), destination.join("accounts")] {
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
            }
        }
        self.control_repository()
            .snapshot_sqlite(&destination.join("control.sqlite"))
            .await?;
        let mut accounts = vec![];
        for (user, account) in self.control_repository().all_accounts().await? {
            let id = account.id;
            let scope = self.authorize(&user, &id).await?;
            self.account_repository(&scope)
                .await?
                .snapshot_sqlite(&destination.join("accounts").join(format!("{id}.sqlite")))
                .await?;
            accounts.push(id);
        }
        copy_tree(&self.root.join("durable"), &destination.join("durable")).await?;
        let manifest = Manifest {
            format: 1,
            created_at: now(),
            accounts,
            jobs_disabled: true,
        };
        // Manifest is written last. Incomplete backups cannot restore.
        tokio::fs::write(
            destination.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )
        .await?;
        Ok(())
    }
    pub async fn restore(
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        key: [u8; 32],
    ) -> Result<Self> {
        let source = source.as_ref();
        let destination = destination.as_ref();
        if destination.exists() {
            return Err(Error::Invalid);
        }
        let manifest: Manifest =
            serde_json::from_slice(&tokio::fs::read(source.join("manifest.json")).await?)?;
        if manifest.format != 1 || !manifest.jobs_disabled {
            return Err(Error::Invalid);
        }
        // Validate names before using any manifest path.
        for id in &manifest.accounts {
            Uuid::parse_str(id).map_err(|_| Error::Invalid)?;
        }
        tokio::fs::create_dir_all(destination.join("accounts")).await?;
        tokio::fs::copy(
            source.join("control.sqlite"),
            destination.join("control.sqlite"),
        )
        .await?;
        for id in &manifest.accounts {
            tokio::fs::copy(
                source.join("accounts").join(format!("{id}.sqlite")),
                destination.join("accounts").join(format!("{id}.sqlite")),
            )
            .await?;
        }
        copy_tree(&source.join("durable"), &destination.join("durable")).await?;
        let store = Self::open(destination, key).await?;
        store.control_repository().reset_restored_auth().await?;
        for id in &manifest.accounts {
            let user = store.control_repository().account_owner(id).await?;
            let a = store.authorize(&user, id).await?;
            // Verify the separately supplied key before allowing operator handover.
            store.credential(&a).await?;
            let db = store.db(&a).await?;
            // Derived mail search must not resurrect provider-deleted content
            // from a snapshot. Rebuild it only after provider reconciliation.
            sqlx::query("DELETE FROM provider_documents WHERE key='search:database'")
                .execute(&db)
                .await?;
            sqlx::query("DELETE FROM search_documents")
                .execute(&db)
                .await?;
            sqlx::query("INSERT OR IGNORE INTO search_dirty SELECT id FROM messages")
                .execute(&db)
                .await?;
            sqlx::query("UPDATE folder_scans SET completed_at=NULL,last_scanned_at=0,continuation=NULL,generation=generation+1,coverage_limited=0").execute(&db).await?;
            sqlx::query("UPDATE provider_documents SET document=json_set(document,'$.refreshed',0) WHERE key='sync:folders' AND json_valid(document)").execute(&db).await?;
            let ids: Vec<String> =
                sqlx::query_scalar("SELECT id FROM submissions WHERE state='submitting'")
                    .fetch_all(&db)
                    .await?;
            for id in ids {
                store
                    .transition(
                        &a,
                        &id,
                        SubmissionState::Submitting,
                        SubmissionState::DeliveryUnknown,
                    )
                    .await?;
            }
        }
        Ok(store)
    }
    pub async fn jobs_enabled(&self) -> Result<bool> {
        let value = self.control_repository().setting("jobs_enabled").await?;
        Ok(value.as_deref() != Some("false"))
    }
}
impl Store {
    /// Offline backend transfer. Preserve durable state and ciphertext exactly;
    /// pause workers and rebuild search instead of carrying a derived index.
    pub async fn import_backend(
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        key: [u8; 32],
        url: Option<&str>,
    ) -> Result<Self> {
        let source = source.as_ref();
        let destination = destination.as_ref();
        if destination.exists() {
            return Err(Error::Invalid);
        }
        let manifest: Manifest =
            serde_json::from_slice(&tokio::fs::read(source.join("manifest.json")).await?)?;
        if manifest.format != 1 || !manifest.jobs_disabled {
            return Err(Error::Invalid);
        }
        let mut ids = manifest.accounts.clone();
        ids.sort();
        ids.dedup();
        if ids.len() != manifest.accounts.len() {
            return Err(Error::Invalid);
        }
        for id in &ids {
            Uuid::parse_str(id).map_err(|_| Error::Invalid)?;
        }
        let mut files = vec![PathBuf::from("control.sqlite")];
        files.extend(
            ids.iter()
                .map(|id| PathBuf::from("accounts").join(format!("{id}.sqlite"))),
        );
        for path in &files {
            if !tokio::fs::symlink_metadata(source.join(path))
                .await?
                .is_file()
            {
                return Err(Error::Invalid);
            }
        }
        tokio::fs::create_dir_all(destination).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o700)).await?;
        }
        // A failed transfer leaves a visible durable guard, so startup cannot
        // quietly use a partial import. No existing destination is overwritten.
        let marker = destination.join("backend-import-in-progress");
        tokio::fs::write(&marker, b"incomplete\n").await?;
        let staging = destination.join("import-staging");
        tokio::fs::create_dir_all(staging.join("accounts")).await?;
        for path in &files {
            tokio::fs::copy(source.join(path), staging.join(path)).await?;
        }
        let staged = Store::open(&staging, key).await?;
        let accounts = staged.control_repository().all_accounts().await?;
        let mut actual: Vec<_> = accounts
            .iter()
            .map(|(_, account)| account.id.clone())
            .collect();
        actual.sort();
        if actual != ids {
            return Err(Error::Invalid);
        }
        // Verify the supplied key against every original AAD before writing any
        // external destination. The archive itself remains untouched.
        for (user, account) in &accounts {
            let a = staged.authorize(user, &account.id).await?;
            staged.credential(&a).await?;
        }
        let postgres = url.is_some_and(|value| {
            value.starts_with("postgresql://") || value.starts_with("postgres://")
        });
        let mut imported = if postgres {
            let lease = account_repository::DatabaseLease::acquire(url.unwrap(), true).await?;
            let control =
                control_repository::ControlRepository::open_postgres(url.unwrap()).await?;
            control
                .import_sqlite(&staging.join("control.sqlite"))
                .await?;
            let mut store =
                Store::from_control(destination.to_owned(), key, 1024 * 1024 * 1024, control)
                    .await?;
            store.application_database = Some(
                account_repository::ApplicationDatabase::open_with_lease(url.unwrap(), lease)
                    .await?,
            );
            store.default_search_url = Some(Arc::from(url.unwrap()));
            for (user, account) in &accounts {
                let a = store.authorize(user, &account.id).await?;
                let repository = store.account_repository(&a).await?;
                repository.initialize_account().await?;
                repository
                    .import_sqlite(
                        &staging
                            .join("accounts")
                            .join(format!("{}.sqlite", account.id)),
                    )
                    .await?;
            }
            store
        } else {
            let path = match url {
                None => destination.join("control.sqlite"),
                Some(value) if value.starts_with("sqlite:") => {
                    let options = SqliteConnectOptions::from_str(value)?;
                    if options.get_filename() == Path::new(":memory:") {
                        return Err(Error::Invalid);
                    }
                    options.get_filename().to_owned()
                }
                Some(_) => return Err(Error::Invalid),
            };
            if path.exists() {
                return Err(Error::Invalid);
            }
            staged.control_repository().snapshot_sqlite(&path).await?;
            tokio::fs::create_dir_all(destination.join("accounts")).await?;
            for (user, account) in &accounts {
                let a = staged.authorize(user, &account.id).await?;
                staged
                    .account_repository(&a)
                    .await?
                    .snapshot_sqlite(
                        &destination
                            .join("accounts")
                            .join(format!("{}.sqlite", account.id)),
                    )
                    .await?;
            }
            let control = control_repository::ControlRepository::sqlite(pool(&path).await?);
            control.put_setting("backend_import", "incomplete").await?;
            Store::from_control(destination.to_owned(), key, 1024 * 1024 * 1024, control).await?
        };
        copy_tree(&source.join("durable"), &destination.join("durable")).await?;
        imported
            .control_repository()
            .put_setting("jobs_enabled", "false")
            .await?;
        // End every temporary SQLite connection before deleting staging files.
        for pool in staged.account_pools.lock().await.values() {
            pool.close().await;
        }
        staged.control_repository().close_for_archive().await;
        tokio::fs::remove_dir_all(&staging).await?;
        imported.configure_search_database(None).await?;
        imported
            .control_repository()
            .put_setting("backend_import", "complete")
            .await?;
        tokio::fs::remove_file(marker).await?;
        Ok(imported)
    }
}
