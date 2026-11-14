//! Account-scoped eligibility and constant-time text progress counters.
use super::*;

impl Store {
    pub async fn search_excluded_folders(&self, a: &AuthorizedAccount) -> Result<Vec<String>> {
        self.account_repository(a)
            .await?
            .search_excluded_folders()
            .await
    }

    /// Call after role discovery or a folder mapping change. Queue both removals
    /// and restorations atomically, without fetching any content.
    pub async fn set_search_excluded_folders(
        &self,
        a: &AuthorizedAccount,
        folders: &[String],
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .set_search_excluded_folders(folders)
            .await
    }

    pub async fn search_message_excluded(
        &self,
        a: &AuthorizedAccount,
        message: &Message,
    ) -> Result<bool> {
        if message.account_id != a.id() {
            return Err(Error::NotFound);
        }
        let names = self.search_excluded_folders(a).await?;
        Ok(message.folder_ids.is_empty()
            || message
                .folder_ids
                .iter()
                .any(|f| names.iter().any(|n| n.eq_ignore_ascii_case(f))))
    }
}
