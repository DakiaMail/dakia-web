use super::*;
impl Store {
    /// Claim once. Lost in-flight work must be reconciled instead of replayed.
    pub async fn claim_mutation(&self, a: &AuthorizedAccount) -> Result<Option<Operation>> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let active = self
            .rules(a.user_id())
            .await?
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| r.id)
            .collect::<Vec<_>>();
        self.account_repository(a)
            .await?
            .claim_mutation(&active)
            .await
    }
    /// Admit retries only after the provider proves that no write was attempted.
    pub async fn retry_mutation(&self, a: &AuthorizedAccount, id: &str) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.retry_mutation(id).await
    }
    pub async fn finish_mutation(
        &self,
        a: &AuthorizedAccount,
        operation: Operation,
        folders: Option<Vec<String>>,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .finish_mutation(operation, folders)
            .await
    }
    pub async fn recover_mutations(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.recover_mutations().await
    }
    pub async fn link_identity(
        &self,
        a: &AuthorizedAccount,
        provider: &str,
        message: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .link_identity(provider, message)
            .await
    }
}
