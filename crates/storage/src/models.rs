use super::*;
pub(crate) const CLASSIFICATION_CANDIDATE_QUERY: &str = "SELECT m.document FROM messages m WHERE json_extract(m.document,'$.classification.status')='pending' AND (instr(upper(json_extract(m.document,'$.folder_ids')),'\"INBOX\"')>0 OR EXISTS(SELECT 1 FROM message_classifications c WHERE c.message_id=m.id AND c.requested=1)) AND NOT EXISTS (SELECT 1 FROM model_jobs WHERE json_extract(model_jobs.document,'$.message_id')=m.id AND state IN ('pending','running')) ORDER BY json_extract(m.document,'$.received_at') DESC LIMIT 1";
#[derive(Debug, Clone, Copy)]
pub enum ModelFailureReason {
    ContentUnavailable,
    ContentInvalid,
    RevisionChanged,
    WorkerFailed,
    StorageUnavailable,
    Deadline,
}
impl ModelFailureReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::ContentUnavailable => "content_unavailable",
            Self::ContentInvalid => "content_invalid",
            Self::RevisionChanged => "revision_changed",
            Self::WorkerFailed => "worker_failed",
            Self::StorageUnavailable => "storage_unavailable",
            Self::Deadline => "deadline",
        }
    }
}
impl Store {
    pub async fn model_failure_reason(
        &self,
        a: &AuthorizedAccount,
        id: &str,
    ) -> Result<Option<String>> {
        self.account_repository(a)
            .await?
            .model_failure_reason(id)
            .await
    }
    pub async fn defer_model_failure(
        &self,
        a: &AuthorizedAccount,
        job: &ModelJob,
        reason: ModelFailureReason,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .defer_model_failure(job, reason)
            .await
    }
    pub async fn model_job(&self, a: &AuthorizedAccount, id: &str) -> Result<ModelJob> {
        self.account_repository(a).await?.model_job(id).await
    }
    pub async fn queue_model(
        &self,
        a: &AuthorizedAccount,
        message: &str,
        kind: JobKind,
        revision: &str,
        source: Option<&str>,
    ) -> Result<ModelJob> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .queue_model(message, kind, revision, source)
            .await
    }
    pub async fn claim_model(&self, a: &AuthorizedAccount) -> Result<Option<ModelJob>> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.claim_model().await
    }
    pub async fn classification_candidate(&self, a: &AuthorizedAccount) -> Result<Option<Message>> {
        let mut timing = timing::Timing::new("classification_candidate");
        let result = self
            .account_repository(a)
            .await?
            .classification_candidate()
            .await;
        timing.stage("classification_decode");
        result
    }
    pub async fn finish_model(
        &self,
        a: &AuthorizedAccount,
        job: ModelJob,
        category: Option<&str>,
        translation: Option<(&str, &MessageContent)>,
    ) -> Result<()> {
        let classification = category.map(classification::legacy_classification);
        self.finish_classification(a, job, classification.as_ref(), translation)
            .await
    }
    pub async fn finish_classification(
        &self,
        a: &AuthorizedAccount,
        job: ModelJob,
        classification: Option<&Classification>,
        translation: Option<(&str, &MessageContent)>,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .finish_model(job, classification, translation)
            .await
    }
    pub async fn recover_models(&self, a: &AuthorizedAccount) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.recover_models().await
    }
}
