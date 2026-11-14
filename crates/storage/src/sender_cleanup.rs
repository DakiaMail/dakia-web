//! A frozen sender-cleanup batch groups the existing durable mutation outbox.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupTarget {
    pub id: String,
    pub expected_revision: u32,
    /// Flags and classification may change while the owner reviews the batch.
    /// A location snapshot permits that drift, never a different message or move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_location: Option<CleanupLocation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupLocation {
    pub provider_id: String,
    pub folder_ids: Vec<String>,
    pub sender_email: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRequest {
    pub id: String,
    pub source_id: String,
    pub targets: Vec<CleanupTarget>,
    pub search_complete: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CleanupJob {
    pub request: CleanupRequest,
    pub account_id: String,
    pub sender: String,
    pub created_at: String,
    pub dismissed: bool,
    pub applied: usize,
    pub failed: usize,
    pub pending: usize,
    #[serde(default)]
    pub failed_targets: Vec<String>,
    /// Response-only optimistic metadata. Never a provider outcome or stored
    /// membership; allows other/reloaded clients to show pending mail in Trash.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projected_rows: Vec<Message>,
}
impl Store {
    pub async fn enqueue_sender_cleanup(
        &self,
        a: &AuthorizedAccount,
        request: CleanupRequest,
    ) -> Result<CleanupJob> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .enqueue_sender_cleanup(request)
            .await
    }
    pub async fn sender_cleanup_jobs(&self, a: &AuthorizedAccount) -> Result<Vec<CleanupJob>> {
        self.account_repository(a)
            .await?
            .sender_cleanup_jobs()
            .await
    }
    pub async fn dismiss_sender_cleanup(&self, a: &AuthorizedAccount, id: &str) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .dismiss_sender_cleanup(id)
            .await
    }
}
