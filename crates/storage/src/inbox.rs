use super::*;

pub(crate) const INBOX_PREDICATE: &str = r#"AND instr(upper(json_extract(m.document,'$.folder_ids')),'"INBOX"')>0
             AND EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE upper(f.value)='INBOX')
             AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             AND NOT EXISTS(SELECT 1 FROM operations o WHERE json_extract(o.document,'$.target_id')=m.id AND json_extract(o.document,'$.status')='pending' AND json_extract(o.document,'$.change.kind') IN ('archive','spam','trash'))
             AND NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?)"#;

pub(crate) const ATTENTION_PREDICATE: &str = "AND (json_extract(m.document,'$.classification.attention_override')=1 OR json_array_length(m.document,'$.classification.attention')>0)";

impl Store {
    /// Current account-local projection, with date keysets and current intent.
    pub async fn inbox_rows(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
    ) -> Result<Vec<Message>> {
        self.inbox_rows_filtered(a, before, category, &ClassificationFilter::default())
            .await
    }
    pub async fn inbox_rows_filtered(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
        filter: &ClassificationFilter,
    ) -> Result<Vec<Message>> {
        self.account_repository(a)
            .await?
            .inbox_rows(before, category, filter)
            .await
    }
    /// Cached rows do not prove provider coverage. Errors retain usable rows.
    pub async fn inbox_coverage(&self, a: &AuthorizedAccount) -> Result<AccountCoverage> {
        self.account_repository(a).await?.inbox_coverage().await
    }
    pub async fn seed_inbox_memberships(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .seed_inbox_memberships()
            .await
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
        self.account_repository(a)
            .await?
            .clear_provider_document("inbox:refresh-error")
            .await
    }
}
