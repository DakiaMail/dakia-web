use super::*;

impl Store {
    /// Snoozes are authoritative local state; opening the view needs no provider scan.
    pub async fn snoozed_rows_filtered(
        &self,
        a: &AuthorizedAccount,
        before: Option<(&str, &str)>,
        category: Option<&str>,
        filter: &ClassificationFilter,
    ) -> Result<Vec<Message>> {
        self.account_repository(a)
            .await?
            .snoozed_rows_filtered(before, category, filter)
            .await
    }

    /// Snooze state is local, authoritative and scoped to an authorized account.
    pub async fn active_snoozes(&self, a: &AuthorizedAccount) -> Result<Vec<serde_json::Value>> {
        self.account_repository(a).await?.active_snoozes().await
    }
}
