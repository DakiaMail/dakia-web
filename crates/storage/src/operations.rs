use super::*;
impl Store {
    pub async fn operation_status(&self, a: &AuthorizedAccount, id: &str) -> Result<Operation> {
        self.account_repository(a).await?.operation_status(id).await
    }

    /// Project queued intent without persisting invented provider memberships.
    /// The worker always reads the original provider location with message().
    pub async fn visible_message(
        &self,
        a: &AuthorizedAccount,
        message: Message,
    ) -> Result<Message> {
        self.account_repository(a)
            .await?
            .visible_message(message)
            .await
    }

    pub async fn upsert_message(&self, a: &AuthorizedAccount, message: Message) -> Result<Message> {
        let mut rows = self.ingest_messages(a, vec![message], &[]).await?;
        Ok(rows.remove(0))
    }

    /// One commit per page, retaining revisions, categories and pending intents.
    pub async fn ingest_messages(
        &self,
        a: &AuthorizedAccount,
        messages: Vec<Message>,
        documents: &[(String, String)],
    ) -> Result<Vec<Message>> {
        let _guard = self.gate.read().await;
        let _rules_guard = self.rules_gate.read().await;
        let rules = self.rules(a.user_id()).await?;
        let connected = self
            .control_repository()
            .account_connected_at(a.user_id(), a.id())
            .await?;
        self.account_repository(a)
            .await?
            .ingest_messages(messages, documents, &rules, connected.as_deref())
            .await
    }

    pub async fn message(&self, a: &AuthorizedAccount, id: &str) -> Result<Message> {
        self.account_repository(a).await?.message(id).await
    }
    pub async fn catalogue(&self, a: &AuthorizedAccount) -> Result<Vec<Message>> {
        self.account_repository(a).await?.catalogue().await
    }
    pub async fn operation(
        &self,
        a: &AuthorizedAccount,
        operation: Operation,
    ) -> Result<Operation> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.operation(operation).await
    }
}
