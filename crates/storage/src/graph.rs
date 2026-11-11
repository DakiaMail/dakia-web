//! Account-local Graph checkpoints and identity repair; never infer removals from list pages.
use super::*;

impl Store {
    /// Cursor and removals commit only after all page observations have been persisted.
    /// Replaying observations after a crash is safe; skipping them is not.
    pub async fn commit_graph_page(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
        rows: &[Message],
        removed: &[String],
        cursor: &str,
        initial: bool,
        complete: bool,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        if rows.len() + removed.len() > 200
            || rows.iter().any(|m| m.account_id != a.id)
            || cursor.len() > 64 * 1024
        {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO folder_scans(folder,initial_boundary) VALUES(?,?)")
            .bind(folder)
            .bind(Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;
        let generation: i64 =
            sqlx::query_scalar("SELECT generation FROM folder_scans WHERE folder=?")
                .bind(folder)
                .fetch_one(&mut *tx)
                .await?;
        self.observe_memberships(&mut tx, a, folder, generation, rows)
            .await?;
        for provider_id in removed {
            let observation: Option<(String,String)> = sqlx::query_as("SELECT m.document,r.document FROM messages m JOIN message_identities i ON i.message_id=m.id JOIN provider_documents r ON r.key='reference:'||m.id WHERE i.provider_id=?")
                .bind(format!("graph:{provider_id}")).fetch_optional(&mut *tx).await?;
            let Some((raw, reference)) = observation else {
                continue;
            };
            let mut message: Message = serde_json::from_str(&raw)?;
            let reference: serde_json::Value = serde_json::from_str(&reference)?;
            if reference["graph_id"].as_str() != Some(provider_id)
                && reference["folder"].as_str() == Some(folder)
            {
                continue; // A newer locator was positively observed in this same folder.
            }
            if message.folder_ids.iter().any(|f| f == folder) {
                message.folder_ids.retain(|f| f != folder);
                Self::upsert_message_tx(&mut tx, message.clone()).await?;
            }
            sqlx::query("DELETE FROM folder_memberships WHERE folder=? AND message_id=?")
                .bind(folder)
                .bind(&message.id)
                .execute(&mut *tx)
                .await?;
        }
        if initial && complete {
            self.finish_snapshot(&mut tx, folder, generation).await?;
        }
        if complete {
            sqlx::query("UPDATE folder_scans SET continuation=NULL,completed_at=?,last_scanned_at=?,coverage_limited=0,uidvalidity=1 WHERE folder=?")
                .bind(Utc::now().timestamp()).bind(Utc::now().timestamp()).bind(folder).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document")
            .bind(format!("graph:cursor:{folder}")).bind(cursor).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    /// A unique matching old observation is only a candidate, never enough to merge a copy.
    pub async fn graph_identity_candidate(
        &self,
        a: &AuthorizedAccount,
        rfc_id: &str,
        fingerprint: &str,
        new_id: &str,
    ) -> Result<Option<(String, String)>> {
        let db = self.db(a).await?;
        let candidates: Vec<(String,String)> = sqlx::query_as("SELECT m.id,json_extract(r.document,'$.graph_id') FROM thread_edges e JOIN messages m ON m.id=e.message_id JOIN provider_documents h ON h.key='thread-headers:'||m.id JOIN provider_documents r ON r.key='reference:'||m.id WHERE e.edge=? AND json_extract(m.document,'$.provider_id') LIKE 'graph:%' AND json_extract(h.document,'$.fingerprint')=? AND json_extract(r.document,'$.graph_id') IS NOT NULL AND json_extract(r.document,'$.graph_id')<>? LIMIT 2")
            .bind(format!("rfc:{rfc_id}")).bind(fingerprint).bind(new_id).fetch_all(&db).await?;
        Ok(if candidates.len() == 1 {
            candidates.into_iter().next()
        } else {
            None
        })
    }
    /// Start/restart an enumeration without advancing a cursor or deleting saved mail.
    pub async fn begin_graph_scan(&self, a: &AuthorizedAccount, folder: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO folder_scans(folder,initial_boundary) VALUES(?,?)")
            .bind(folder)
            .bind(Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=0 WHERE folder=?").bind(folder).execute(&mut *tx).await?;
        self.seed_projected_memberships(&mut tx, folder).await?;
        sqlx::query("DELETE FROM provider_documents WHERE key=?")
            .bind(format!("graph:cursor:{folder}"))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
