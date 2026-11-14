use super::*;
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
