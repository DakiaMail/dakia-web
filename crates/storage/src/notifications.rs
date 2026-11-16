use super::*;

impl Store {
    pub async fn subscribe(&self, user: &str, id: &str, document: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        self.control_repository()
            .subscribe(user, id, document)
            .await?;
        Ok(())
    }
    pub async fn push_subscription_enabled(&self, user: &str, id: &str) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.control_repository()
            .push_subscription_enabled(user, id)
            .await
    }
    pub async fn unsubscribe_push(&self, user: &str, id: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        self.control_repository()
            .expire_subscription(user, id)
            .await
    }
    pub async fn subscriptions(&self, a: &AuthorizedAccount) -> Result<Vec<serde_json::Value>> {
        self.authorize(&a.user_id, &a.id).await?;
        self.control_repository().subscriptions(&a.user_id).await
    }
    pub async fn expire_subscription(&self, a: &AuthorizedAccount, id: &str) -> Result<()> {
        self.authorize(&a.user_id, &a.id).await?;
        self.control_repository()
            .expire_subscription(&a.user_id, id)
            .await?;
        Ok(())
    }
    pub async fn next_push(&self, a: &AuthorizedAccount) -> Result<Option<serde_json::Value>> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let rules = self.rules(a.user_id()).await?;
        self.account_repository(a).await?.next_push(&rules).await
    }
    pub async fn push_send_allowed(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        subscription: &str,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let rules = self.rules(a.user_id()).await?;
        self.account_repository(a)
            .await?
            .push_send_allowed(message, subscription, &rules)
            .await
    }
    pub async fn finish_push(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        subscription: &str,
        retry: Option<u32>,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .finish_push(message, subscription, retry)
            .await
    }
    pub async fn next_push_claim(
        &self,
        a: &AuthorizedAccount,
    ) -> Result<Option<(serde_json::Value, i64)>> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let rules = self.rules(a.user_id()).await?;
        self.account_repository(a)
            .await?
            .next_push_claim(&rules)
            .await
    }
    pub async fn finish_push_attempt(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        subscription: &str,
        attempt: i64,
        retry: Option<u32>,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .finish_push_attempt(message, subscription, attempt, retry)
            .await
    }
    pub async fn recover_push(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let rules = self.rules(a.user_id()).await?;
        self.account_repository(a).await?.recover_push(&rules).await
    }
    pub async fn is_snoozed(&self, a: &AuthorizedAccount, thread: &str) -> Result<bool> {
        self.account_repository(a).await?.is_snoozed(thread).await
    }
    pub async fn wake_snoozes(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.wake_snoozes().await
    }
}
