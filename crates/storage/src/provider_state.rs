use super::*;
impl Store {
    pub async fn account_state(
        &self,
        a: &AuthorizedAccount,
        state: AccountState,
    ) -> Result<Account> {
        let _guard = self.gate.read().await;
        self.authorize(&a.user_id, &a.id).await?;
        self.control_repository()
            .mutate_account(&a.user_id, &a.id, None, |account| {
                account.state = state;
                account.revision = account.revision.checked_add(1).ok_or(Error::Invalid)?;
                Ok(())
            })
            .await
    }
    pub async fn put_provider_document(
        &self,
        a: &AuthorizedAccount,
        key: &str,
        document: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .put_provider_document(key, document)
            .await
    }
    /// Bind unsubscribe intent and immutable SMTP work in one transaction.
    pub async fn enqueue_unsubscribe(
        &self,
        a: &AuthorizedAccount,
        key: &str,
        draft: Draft,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .enqueue_unsubscribe(key, draft)
            .await
    }
    pub async fn claim_unsubscribe(&self, a: &AuthorizedAccount, key: &str) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .claim_unsubscribe(key)
            .await
    }
    /// Bounded exact-prefix lookup for previously scanned memberships.
    pub async fn message_reference_documents(
        &self,
        a: &AuthorizedAccount,
        message_id: &str,
    ) -> Result<Vec<String>> {
        self.account_repository(a)
            .await?
            .message_reference_documents(message_id)
            .await
    }
    pub async fn provider_document(
        &self,
        a: &AuthorizedAccount,
        key: &str,
    ) -> Result<Option<String>> {
        self.account_repository(a)
            .await?
            .provider_document(key)
            .await
    }
}
