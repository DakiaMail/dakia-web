use super::*;
#[cfg(test)]
#[path = "thread_graph_tests.rs"]
mod tests;

impl Store {
    pub async fn thread_revision(&self, a: &AuthorizedAccount) -> Result<i64> {
        let db = self.db(a).await?;
        Ok(
            sqlx::query_scalar("SELECT revision FROM thread_generation WHERE id=1")
                .fetch_one(&db)
                .await?,
        )
    }
    pub async fn thread_verified(
        &self,
        a: &AuthorizedAccount,
        component: &str,
        revision: i64,
    ) -> Result<bool> {
        let db = self.db(a).await?;
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM thread_verifications v JOIN thread_generation g ON g.id=1 WHERE v.component=? AND v.revision=? AND g.revision=v.revision AND v.checked_at>?)")
            .bind(component).bind(revision).bind(Utc::now().timestamp()-30).fetch_one(&db).await?)
    }
    pub async fn verify_thread(&self, a: &AuthorizedAccount, component: &str) -> Result<()> {
        self.verify_thread_revision(a, component, self.thread_revision(a).await?)
            .await
            .map(|_| ())
    }
    pub async fn verify_thread_revision(
        &self,
        a: &AuthorizedAccount,
        component: &str,
        revision: i64,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let verified = sqlx::query("INSERT INTO thread_verifications(component,revision,checked_at) SELECT ?,revision,? FROM thread_generation WHERE id=1 AND revision=? ON CONFLICT(component) DO UPDATE SET revision=excluded.revision,checked_at=excluded.checked_at")
            .bind(component).bind(Utc::now().timestamp()).bind(revision).execute(&mut *tx).await?.rows_affected() > 0;
        sqlx::query("DELETE FROM thread_verifications WHERE component NOT IN (SELECT component FROM thread_verifications ORDER BY checked_at DESC,component LIMIT 128)").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(verified)
    }
    /// Bounded indexed lookup. Join current documents instead of cached API rows:
    /// deleted memberships, manual categories and current flags remain authoritative.
    pub async fn thread_neighbors(
        &self,
        a: &AuthorizedAccount,
        edges: &[String],
    ) -> Result<Vec<(Message, String)>> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }
        if edges.len() > 256 {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT DISTINCT m.document,d.document FROM thread_edges e JOIN messages m ON m.id=e.message_id JOIN provider_documents d ON d.key='thread-headers:'||m.id WHERE e.edge IN (",
        );
        let mut values = q.separated(",");
        for edge in edges {
            values.push_bind(edge);
        }
        values.push_unseparated(") AND json_array_length(m.document,'$.folder_ids')>0
          AND (NOT EXISTS(SELECT 1 FROM provider_documents WHERE key='thread:folders')
            OR EXISTS(SELECT 1 FROM provider_documents f,json_each(f.document,'$.names') n,json_each(m.document,'$.folder_ids') mf WHERE f.key='thread:folders' AND n.value=mf.value))
          AND (json_extract(m.document,'$.provider_id') LIKE 'gmail:%'
            OR NOT json_valid(json_extract(m.document,'$.provider_id'))
            OR NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')))
          ORDER BY m.id LIMIT 1001");
        let rows: Vec<(String, String)> = q.build_query_as().fetch_all(&db).await?;
        rows.into_iter()
            .map(|(m, h)| Ok((serde_json::from_str(&m)?, h)))
            .collect()
    }

    /// A portable full metadata reconciliation is the coverage authority. New UID
    /// scans alone cannot establish absence of deletions or changed flags.
    pub async fn thread_coverage_fresh(&self, a: &AuthorizedAccount) -> Result<bool> {
        let db = self.db(a).await?;
        let cutoff = Utc::now().timestamp() - 300;
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM provider_documents f JOIN provider_documents p ON p.key='probe' WHERE f.key='thread:folders' AND json_extract(f.document,'$.at')>? AND json_extract(p.document,'$.full_search_coverage')=1 AND json_array_length(f.document,'$.names')>0 AND NOT EXISTS(SELECT 1 FROM json_each(f.document,'$.names') n LEFT JOIN folder_scans s ON s.folder=n.value LEFT JOIN provider_documents v ON v.key='thread:epoch:'||n.value WHERE s.completed_at IS NULL OR s.completed_at<=? OR s.coverage_limited=1 OR s.uidvalidity IS NULL OR v.document IS NULL OR s.uidvalidity IS NOT CAST(v.document AS INTEGER)))")
            .bind(cutoff).bind(cutoff).fetch_one(&db).await?)
    }
}
