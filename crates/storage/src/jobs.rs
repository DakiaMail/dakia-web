use super::*;
#[derive(Clone)]
pub struct PersistedMime {
    pub raw: Vec<u8>,
    pub recipients: Vec<String>,
    pub message_id: String,
}
impl Store {
    /// Local indexing can continue through provider reconnects using text already
    /// retained in search storage. It never needs provider credentials.
    pub async fn indexing_accounts(&self) -> Result<Vec<AuthorizedAccount>> {
        let mut accounts = Vec::new();
        for (user, metadata) in self.control_repository().all_accounts().await? {
            match self.authorize(&user, &metadata.id).await {
                Ok(a) => accounts.push(a),
                Err(Error::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(accounts)
    }
    pub async fn job_accounts(&self) -> Result<Vec<AuthorizedAccount>> {
        let mut accounts = vec![];
        for (user, metadata) in self.control_repository().all_accounts().await? {
            if metadata.state == AccountState::Ready {
                accounts.push(self.authorize(&user, &metadata.id).await?);
            }
        }
        Ok(accounts)
    }
    pub async fn due_submission(
        &self,
        a: &AuthorizedAccount,
        at: &str,
    ) -> Result<Option<Submission>> {
        if !self.jobs_enabled().await? {
            return Ok(None);
        }
        let id = self.account_repository(a).await?.due_submission(at).await?;
        let Some(id) = id else { return Ok(None) };
        match self
            .transition(a, &id, SubmissionState::Scheduled, SubmissionState::Claimed)
            .await
        {
            Ok(s) => Ok(Some(s)),
            Err(Error::Conflict) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub async fn submission_snapshot(&self, a: &AuthorizedAccount, id: &str) -> Result<Draft> {
        self.account_repository(a)
            .await?
            .submission_snapshot(id)
            .await
    }
    pub async fn mime(&self, a: &AuthorizedAccount, id: &str) -> Result<Option<PersistedMime>> {
        self.account_repository(a).await?.mime(id).await
    }
    pub async fn persist_mime(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        mime: PersistedMime,
    ) -> Result<PersistedMime> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .persist_mime(id, mime)
            .await
    }
    /// Only a transport-proven refusal may schedule another attempt. Unknown deliveries never enter here.
    pub async fn defer_submission(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        from: SubmissionState,
        delay: u64,
    ) -> Result<Submission> {
        if !matches!(from, SubmissionState::Claimed | SubmissionState::Submitting) {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .defer_submission(id, from, delay)
            .await
    }
    pub async fn uncertain_submissions(&self, a: &AuthorizedAccount) -> Result<Vec<Submission>> {
        self.account_repository(a)
            .await?
            .uncertain_submissions()
            .await
    }
    /// Positive provider-side Sent evidence is the only recovery; a negative lookup never reschedules.
    pub async fn confirm_delivery(&self, a: &AuthorizedAccount, id: &str) -> Result<Submission> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.confirm_delivery(id).await
    }
    pub async fn needs_filing(&self, a: &AuthorizedAccount) -> Result<Vec<Submission>> {
        self.account_repository(a).await?.needs_filing().await
    }
    pub async fn set_filing(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        status: FilingStatus,
    ) -> Result<Submission> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .set_filing(id, status)
            .await
    }
}
