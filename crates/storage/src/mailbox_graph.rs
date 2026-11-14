use super::*;
impl AccountRepository {
    pub(crate) async fn thread_revision(&self) -> Result<i64> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT revision FROM thread_generation WHERE id=1",[],"SELECT COALESCE((SELECT thread_revision FROM account_locks WHERE account_id=$1),0)",[&self.account],fetch_one))
        })
    }
    pub(crate) async fn thread_verified(&self, component: &str, revision: i64) -> Result<bool> {
        let cutoff = Utc::now().timestamp() - 30;
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM thread_verifications v JOIN thread_generation g ON g.id=1 WHERE v.component=? AND v.revision=? AND g.revision=v.revision AND v.checked_at>?)",[component,revision,cutoff],"SELECT EXISTS(SELECT 1 FROM thread_verifications v JOIN account_locks g ON g.account_id=v.account_id WHERE v.account_id=$1 AND v.component=$2 AND v.revision=$3 AND g.thread_revision=v.revision AND v.checked_at>$4)",[&self.account,component,revision,cutoff],fetch_one))
        })
    }
    pub(crate) async fn verify_thread_revision(
        &self,
        component: &str,
        revision: i64,
    ) -> Result<bool> {
        let at = Utc::now().timestamp();
        transaction!(self, tx, local, true, {
            let verified=execute!(local,tx,"INSERT INTO thread_verifications(component,revision,checked_at) SELECT ?,revision,? FROM thread_generation WHERE id=1 AND revision=? ON CONFLICT(component) DO UPDATE SET revision=excluded.revision,checked_at=excluded.checked_at",[component,at,revision],"INSERT INTO thread_verifications(account_id,component,revision,checked_at) SELECT $1,$2,thread_revision,$3 FROM account_locks WHERE account_id=$1 AND thread_revision=$4 ON CONFLICT(account_id,component) DO UPDATE SET revision=excluded.revision,checked_at=excluded.checked_at",[&self.account,component,at,revision])>0;
            execute!(local,tx,"DELETE FROM thread_verifications WHERE component NOT IN (SELECT component FROM thread_verifications ORDER BY checked_at DESC,component LIMIT 128)",[],"DELETE FROM thread_verifications WHERE account_id=$1 AND component NOT IN (SELECT component FROM thread_verifications WHERE account_id=$1 ORDER BY checked_at DESC,component COLLATE \"C\" LIMIT 128)",[&self.account]);
            tx.commit().await?;
            Ok(verified)
        })
    }
    pub(crate) async fn thread_neighbors(
        &self,
        edges: &[String],
    ) -> Result<Vec<(Message, String)>> {
        if edges.is_empty() {
            return Ok(vec![]);
        }
        if edges.len() > 256 {
            return Err(Error::Invalid);
        }
        let rows: Vec<(String, String)> = match &self.backend {
            Backend::Sqlite(db) => {
                let mut tx = db.begin().await?;
                let mut query=sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT DISTINCT m.document,d.document FROM thread_edges e JOIN messages m ON m.id=e.message_id JOIN provider_documents d ON d.key='thread-headers:'||m.id WHERE e.edge IN (");
                let mut values = query.separated(",");
                for edge in edges {
                    values.push_bind(edge);
                }
                values.push_unseparated(") AND json_array_length(m.document,'$.folder_ids')>0 AND (NOT EXISTS(SELECT 1 FROM provider_documents WHERE key='thread:folders') OR EXISTS(SELECT 1 FROM provider_documents f,json_each(f.document,'$.names') n,json_each(m.document,'$.folder_ids') mf WHERE f.key='thread:folders' AND n.value=mf.value)) AND (json_extract(m.document,'$.provider_id') LIKE 'gmail:%' OR NOT json_valid(json_extract(m.document,'$.provider_id')) OR NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity'))) ORDER BY m.id LIMIT 1001");
                let rows = query.build_query_as().fetch_all(&mut *tx).await?;
                tx.commit().await?;
                rows
            }
            Backend::Postgres(db) => {
                let mut tx = self.begin_postgres(db, false).await?;
                let rows=sqlx::query_as("SELECT document,headers FROM (SELECT DISTINCT m.id,m.document,d.document AS headers FROM thread_edges e JOIN messages m ON m.account_id=e.account_id AND m.id=e.message_id JOIN provider_documents d ON d.account_id=m.account_id AND d.key='thread-headers:'||m.id WHERE e.account_id=$1 AND e.edge=ANY($2) AND cardinality(m.folder_ids)>0 AND (NOT EXISTS(SELECT 1 FROM provider_documents WHERE account_id=$1 AND key='thread:folders') OR EXISTS(SELECT 1 FROM provider_documents f CROSS JOIN LATERAL jsonb_array_elements_text(f.document::jsonb->'names') n WHERE f.account_id=$1 AND f.key='thread:folders' AND n=ANY(m.folder_ids))) AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=$1 AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch)) neighbors ORDER BY id COLLATE \"C\" LIMIT 1001")
                    .bind(&self.account).bind(edges).fetch_all(&mut *tx).await?;
                tx.commit().await?;
                rows
            }
        };
        rows.into_iter()
            .map(|(message, headers)| Ok((serde_json::from_str(&message)?, headers)))
            .collect()
    }
    pub(crate) async fn thread_coverage_fresh(&self) -> Result<bool> {
        let cutoff = Utc::now().timestamp() - 300;
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM provider_documents f JOIN provider_documents p ON p.key='probe' WHERE f.key='thread:folders' AND json_extract(f.document,'$.at')>? AND json_extract(p.document,'$.full_search_coverage')=1 AND json_array_length(f.document,'$.names')>0 AND NOT EXISTS(SELECT 1 FROM json_each(f.document,'$.names') n LEFT JOIN folder_scans s ON s.folder=n.value LEFT JOIN provider_documents v ON v.key='thread:epoch:'||n.value WHERE s.completed_at IS NULL OR s.completed_at<=? OR s.coverage_limited=1 OR s.uidvalidity IS NULL OR v.document IS NULL OR s.uidvalidity IS NOT CAST(v.document AS INTEGER)))",[cutoff,cutoff],"SELECT EXISTS(SELECT 1 FROM provider_documents f JOIN provider_documents p ON p.account_id=f.account_id AND p.key='probe' WHERE f.account_id=$1 AND f.key='thread:folders' AND (f.document::jsonb->>'at')::BIGINT>$2 AND p.document::jsonb->'full_search_coverage'='true'::jsonb AND jsonb_array_length(f.document::jsonb->'names')>0 AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements_text(f.document::jsonb->'names') n LEFT JOIN folder_scans s ON s.account_id=$1 AND s.folder=n LEFT JOIN provider_documents v ON v.account_id=$1 AND v.key='thread:epoch:'||n WHERE s.completed_at IS NULL OR s.completed_at<=$2 OR s.coverage_limited OR s.uidvalidity IS NULL OR v.document IS NULL OR s.uidvalidity IS DISTINCT FROM v.document::BIGINT))",[&self.account,cutoff],fetch_one))
        })
    }
}
