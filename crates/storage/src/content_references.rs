//! Read failures block exact Gmail references, not the stable Gmail identity.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct ContentReferences {
    pub token: String,
    pub source: String,
    pub blocked: Vec<String>,
    #[serde(default)]
    pub blocked_at: i64,
}
impl Store {
    pub async fn content_references(
        &self,
        a: &AuthorizedAccount,
        message: &Message,
    ) -> Result<ContentReferences> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .content_references(message)
            .await
    }
    pub async fn block_content_reference(
        &self,
        a: &AuthorizedAccount,
        message: &Message,
        reference: &str,
        token: &str,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .block_content_reference(message, reference, token)
            .await
    }
    pub async fn mark_reference_gap(
        &self,
        a: &AuthorizedAccount,
        message: &Message,
        token: &str,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .mark_reference_gap(message, token)
            .await
    }
    pub async fn promote_content_reference(
        &self,
        a: &AuthorizedAccount,
        message: &Message,
        reference: &str,
        token: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .promote_content_reference(message, reference, token)
            .await
    }
    pub async fn forget_empty_content_references(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        token: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .forget_empty_content_references(id, token)
            .await
    }
    pub async fn claim_reference_recovery(&self, a: &AuthorizedAccount) -> Result<Option<Message>> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .claim_reference_recovery()
            .await
    }
}
