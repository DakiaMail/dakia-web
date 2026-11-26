use super::*;
impl Store {
    pub async fn folder_rows(
        &self,
        a: &AuthorizedAccount,
        folders: &[String],
        before: Option<(&str, &str)>,
    ) -> Result<Vec<Message>> {
        self.account_repository(a)
            .await?
            .folder_rows(folders, before)
            .await
    }
    /// Revoke a contradicted snapshot only while its exact reference is current.
    pub async fn invalidate_stale_reference(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        folder: &str,
        reference: &str,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .invalidate_stale_reference(message, folder, reference)
            .await
    }
    pub async fn scan_generation(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<Option<i64>> {
        self.account_repository(a)
            .await?
            .scan_generation(folder)
            .await
    }
    pub async fn terminal_scan_folders(&self, a: &AuthorizedAccount) -> Result<Vec<String>> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .terminal_scan_folders()
            .await
    }
    /// Terminal folders and whether their provider coverage was limited.
    pub async fn terminal_scan_coverage(
        &self,
        a: &AuthorizedAccount,
    ) -> Result<Vec<(String, bool)>> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .terminal_scan_coverage()
            .await
    }
    pub async fn completed_scan_due(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<Option<i64>> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .completed_scan_due(folder)
            .await
    }
    pub async fn confirm_unchanged_scan(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
        generation: i64,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .confirm_unchanged_scan(folder, generation)
            .await
    }
    pub async fn verified_snapshot_current(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<bool> {
        self.account_repository(a)
            .await?
            .verified_snapshot_current(folder)
            .await
    }
    pub async fn scan_state(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<Option<(i64, Option<String>)>> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.scan_state(folder).await
    }
    pub async fn reset_scan(&self, a: &AuthorizedAccount, folder: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.reset_scan(folder).await
    }
    /// Commit coverage and memberships together. Limited scans never prune.
    pub async fn record_scan(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
        generation: i64,
        page: &SearchPage,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let subscriptions = self.subscriptions(a).await?;
        let rules = self.rules(a.user_id()).await?;
        self.account_repository(a)
            .await?
            .record_scan(folder, generation, page, &subscriptions, &rules)
            .await
    }
    /// Arrival observations and their checkpoint never certify history coverage.
    pub async fn record_incremental(
        &self,
        a: &AuthorizedAccount,
        folder: &str,
        rows: &[Message],
        checkpoint: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let subscriptions = self.subscriptions(a).await?;
        let rules = self.rules(a.user_id()).await?;
        self.account_repository(a)
            .await?
            .record_incremental(folder, rows, checkpoint, &subscriptions, &rules)
            .await
    }
}
