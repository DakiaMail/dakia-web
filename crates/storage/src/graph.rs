//! Account-local Graph checkpoints and identity repair; never infer removals from list pages.
use super::*;

impl Store {
    /// Cursor and removals commit only after all page observations have been persisted.
    /// Replaying observations after a crash is safe; skipping them is not.
    #[allow(clippy::too_many_arguments)]
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
        let subscriptions = self.subscriptions(a).await?;
        let rules = self.rules(a.user_id()).await?;
        self.account_repository(a)
            .await?
            .commit_graph_page(
                folder,
                rows,
                removed,
                cursor,
                initial,
                complete,
                &subscriptions,
                &rules,
            )
            .await
    }
    /// A unique matching old observation is only a candidate, never enough to merge a copy.
    pub async fn graph_identity_candidate(
        &self,
        a: &AuthorizedAccount,
        rfc_id: &str,
        fingerprint: &str,
        new_id: &str,
    ) -> Result<Option<(String, String)>> {
        self.account_repository(a)
            .await?
            .graph_identity_candidate(rfc_id, fingerprint, new_id)
            .await
    }
    /// Start/restart an enumeration without advancing a cursor or deleting saved mail.
    pub async fn begin_graph_scan(&self, a: &AuthorizedAccount, folder: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .begin_graph_scan(folder)
            .await
    }
}
