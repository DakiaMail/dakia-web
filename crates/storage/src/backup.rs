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
        sqlx::query("VACUUM INTO ?")
            .bind(
                destination
                    .join("control.sqlite")
                    .to_string_lossy()
                    .as_ref(),
            )
            .execute(&self.control)
            .await?;
        let rows = sqlx::query("SELECT id,user_id FROM accounts ORDER BY id")
            .fetch_all(&self.control)
            .await?;
        let mut accounts = vec![];
        for row in rows {
            let id: String = row.get("id");
            let scope = self.authorize(row.get("user_id"), &id).await?;
            let db = self.db(&scope).await?;
            sqlx::query("VACUUM INTO ?")
                .bind(
                    destination
                        .join("accounts")
                        .join(format!("{id}.sqlite"))
                        .to_string_lossy()
                        .as_ref(),
                )
                .execute(&db)
                .await?;
            // Search is rebuildable private text, not durable owner state.
            // Remove it from the independent snapshot, including freed pages
            // and WAL, rather than extending its lifetime through backups.
            let snapshot = pool(&destination.join("accounts").join(format!("{id}.sqlite"))).await?;
            sqlx::query("DELETE FROM search_documents")
                .execute(&snapshot)
                .await?;
            sqlx::query("INSERT OR IGNORE INTO search_dirty SELECT id FROM messages")
                .execute(&snapshot)
                .await?;
            sqlx::query("VACUUM").execute(&snapshot).await?;
            sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                .execute(&snapshot)
                .await?;
            snapshot.close().await;
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
        sqlx::query("DELETE FROM sessions")
            .execute(&store.control)
            .await?;
        sqlx::query("DELETE FROM local_invitations")
            .execute(&store.control)
            .await?;
        sqlx::query("DELETE FROM oidc_flows")
            .execute(&store.control)
            .await?;
        sqlx::query("INSERT OR REPLACE INTO settings(key,value) VALUES('jobs_enabled','false')")
            .execute(&store.control)
            .await?;
        for id in &manifest.accounts {
            let user: String = sqlx::query_scalar("SELECT user_id FROM accounts WHERE id=?")
                .bind(id)
                .fetch_one(&store.control)
                .await?;
            let a = store.authorize(&user, id).await?;
            // Verify the separately supplied key before allowing operator handover.
            store.credential(&a).await?;
            let db = store.db(&a).await?;
            // Derived mail search must not resurrect provider-deleted content
            // from a snapshot. Rebuild it only after provider reconciliation.
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
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='jobs_enabled'")
                .fetch_optional(&self.control)
                .await?;
        Ok(value.as_deref() != Some("false"))
    }
}
