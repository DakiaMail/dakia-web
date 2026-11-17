use super::*;
impl Store {
    /// Read immutable send snapshots without changing delivery history.
    pub async fn scheduled_messages(&self, a: &AuthorizedAccount) -> Result<Vec<ScheduledMessage>> {
        self.account_repository(a).await?.scheduled_messages().await
    }
    pub async fn drafts(&self, a: &AuthorizedAccount) -> Result<Vec<Draft>> {
        self.account_repository(a).await?.drafts().await
    }
    pub async fn submissions(&self, a: &AuthorizedAccount) -> Result<Vec<Submission>> {
        self.account_repository(a).await?.submissions().await
    }
}
