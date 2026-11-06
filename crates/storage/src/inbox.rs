use super::*;

impl Store {
    /// Current account-local projection, not a provider response cache. Keyset
    /// pagination survives deletions and new arrivals without offset duplicates.
    pub async fn inbox_rows(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
    ) -> Result<Vec<Message>> {
        let db = self.db(a).await?;
        let rows: Vec<String> = sqlx::query_scalar(
            r#"SELECT m.document FROM messages m
             WHERE instr(upper(json_extract(m.document,'$.folder_ids')),'"INBOX"')>0
             AND EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE upper(f.value)='INBOX')
             AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             AND NOT EXISTS(SELECT 1 FROM operations o WHERE json_extract(o.document,'$.target_id')=m.id AND json_extract(o.document,'$.status')='pending' AND json_extract(o.document,'$.change.kind') IN ('archive','spam','trash'))
             AND NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?)
             AND (? IS NULL OR json_extract(m.document,'$.category') IS NULL OR json_extract(m.document,'$.category')=?)
             AND (? IS NULL OR (json_extract(m.document,'$.received_at'),m.id)<(?,?))
             ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 51"#,
        )
        .bind(now())
        .bind(category).bind(category)
        .bind(before.map(|b| b.0)).bind(before.map(|b| b.0)).bind(before.map(|b| b.1))
        .fetch_all(&db).await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }

    /// Cache coverage is never inferred from row count. A complete scan ages
    /// back to pending, while failures retain usable rows without claiming fresh.
    pub async fn inbox_coverage(&self, a: &AuthorizedAccount) -> Result<AccountCoverage> {
        let db = self.db(a).await?;
        let (completed, limited, continuation, last_scanned_at): (Option<i64>, bool, Option<String>, Option<i64>) =
            sqlx::query_as("SELECT completed_at,coverage_limited,continuation,last_scanned_at FROM folder_scans WHERE folder='INBOX'")
                .fetch_optional(&db).await?.unwrap_or((None,false,None,None));
        let error = self.provider_document(a, "inbox:refresh-error").await?;
        Ok(AccountCoverage {
            account_id: a.id.clone(),
            folder_ids: vec!["INBOX".into()],
            state: if error.is_some() {
                CoverageState::Error
            } else if completed.is_some_and(|t| Utc::now().timestamp() - t < 300) {
                CoverageState::Complete
            } else if limited && continuation.is_none() {
                CoverageState::Partial
            } else {
                CoverageState::Pending
            },
            error,
            inbox: Some(InboxFreshness {
                last_scanned_at,
                refreshing: completed.is_none() && !(limited && continuation.is_none()),
            }),
        })
    }

    /// Include legacy cached rows in reconciliation so an authoritative complete
    /// scan can retire old/deleted/UIDVALIDITY-reset memberships as well.
    /// Check the membership index before decoding legacy message JSON. Repeated
    /// turns must still discover newly cached rows without reparsing every
    /// already-seeded message in a large folder.
    pub async fn seed_inbox_memberships(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        sqlx::query("INSERT OR IGNORE INTO folder_memberships(folder,message_id,generation) SELECT 'INBOX',m.id,-1 FROM messages m WHERE CASE WHEN EXISTS(SELECT 1 FROM folder_memberships known WHERE known.folder='INBOX' AND known.message_id=m.id) THEN 0 ELSE EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE upper(f.value)='INBOX') END")
            .execute(&db).await?;
        Ok(())
    }

    pub async fn inbox_refresh_result(
        &self,
        a: &AuthorizedAccount,
        error: Option<&str>,
    ) -> Result<()> {
        if let Some(error) = error {
            return self
                .put_provider_document(a, "inbox:refresh-error", error)
                .await;
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        sqlx::query("DELETE FROM provider_documents WHERE key='inbox:refresh-error'")
            .execute(&db)
            .await?;
        Ok(())
    }
}
