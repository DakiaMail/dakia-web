use super::*;
#[derive(Clone)]
pub struct PersistedMime {
    pub raw: Vec<u8>,
    pub recipients: Vec<String>,
    pub message_id: String,
}
impl Store {
    pub async fn job_accounts(&self) -> Result<Vec<AuthorizedAccount>> {
        let rows = sqlx::query("SELECT id,user_id,metadata FROM accounts ORDER BY id")
            .fetch_all(&self.control)
            .await?;
        let mut accounts = vec![];
        for r in rows {
            let metadata: Account = serde_json::from_str(r.get("metadata"))?;
            if metadata.state == AccountState::Ready {
                accounts.push(self.authorize(r.get("user_id"), r.get("id")).await?)
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
        let db = self.db(a).await?;
        let id:Option<String>=sqlx::query_scalar("SELECT id FROM submissions WHERE state='scheduled' AND due_at<=? ORDER BY due_at,id LIMIT 1").bind(at).fetch_optional(&db).await?;
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
        let db = self.db(a).await?;
        let raw: Option<String> = sqlx::query_scalar("SELECT snapshot FROM submissions WHERE id=?")
            .bind(id)
            .fetch_optional(&db)
            .await?;
        Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
    }
    pub async fn mime(&self, a: &AuthorizedAccount, id: &str) -> Result<Option<PersistedMime>> {
        let db = self.db(a).await?;
        let r = sqlx::query(
            "SELECT raw,recipients,message_id FROM submission_mime WHERE submission_id=?",
        )
        .bind(id)
        .fetch_optional(&db)
        .await?;
        r.map(|r| {
            Ok(PersistedMime {
                raw: r.get("raw"),
                recipients: serde_json::from_str(r.get("recipients"))?,
                message_id: r.get("message_id"),
            })
        })
        .transpose()
    }
    pub async fn persist_mime(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        mime: PersistedMime,
    ) -> Result<PersistedMime> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        sqlx::query("INSERT OR IGNORE INTO submission_mime(submission_id,raw,recipients,message_id) VALUES(?,?,?,?)").bind(id).bind(mime.raw).bind(serde_json::to_string(&mime.recipients)?).bind(mime.message_id).execute(&db).await?;
        self.mime(a, id).await?.ok_or(Error::NotFound)
    }
    /// Only a transport-proven refusal may schedule another attempt. Unknown deliveries never enter here.
    pub async fn defer_submission(&self, a: &AuthorizedAccount, id: &str, from: SubmissionState, delay: u64) -> Result<Submission> {
        if !matches!(from, SubmissionState::Claimed | SubmissionState::Submitting) { return Err(Error::Invalid); }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE submissions SET state=state WHERE id=?").bind(id).execute(&mut *tx).await?;
        let raw: String = sqlx::query_scalar("SELECT document FROM submissions WHERE id=?").bind(id).fetch_one(&mut *tx).await?;
        let mut submission: Submission = serde_json::from_str(&raw)?;
        if submission.state!=from { return Err(Error::Conflict); }
        let key = format!("send:attempts:{id}");
        let attempts: Option<String> = sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=?").bind(&key).fetch_optional(&mut *tx).await?;
        let attempts = attempts.map(|s| s.parse::<u32>().map_err(|_| Error::Invalid)).transpose()?.unwrap_or(0);
        submission.state = if attempts>=5 { SubmissionState::Failed } else { SubmissionState::Scheduled };
        submission.due_at = (Utc::now()+Duration::seconds(delay.clamp(1,86400).max(5*(1u64 << attempts.min(10))) as i64)).to_rfc3339_opts(SecondsFormat::Millis,true);
        submission.revision += 1;
        let state = serde_json::to_value(&submission.state)?.as_str().ok_or(Error::Invalid)?.to_owned();
        sqlx::query("UPDATE submissions SET state=?,due_at=?,lease_until=NULL,document=? WHERE id=?").bind(state).bind(&submission.due_at).bind(serde_json::to_string(&submission)?).bind(id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document").bind(key).bind((attempts+1).to_string()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'submission_changed',?)").bind(submission.revision).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(submission)
    }
    pub async fn uncertain_submissions(&self, a: &AuthorizedAccount) -> Result<Vec<Submission>> {
        let db = self.db(a).await?;
        let rows: Vec<String> = sqlx::query_scalar("SELECT s.document FROM submissions s WHERE s.state='delivery_unknown' AND COALESCE((SELECT CAST(document AS INTEGER) FROM provider_documents WHERE key='send:lookup:'||s.id),0)<=? ORDER BY COALESCE((SELECT CAST(document AS INTEGER) FROM provider_documents WHERE key='send:lookup:'||s.id),0),due_at LIMIT 10").bind(Utc::now().timestamp()-60).fetch_all(&db).await?;
        rows.into_iter().map(|raw| Ok(serde_json::from_str(&raw)?)).collect()
    }
    /// Positive provider-side Sent evidence is the only recovery; a negative lookup never reschedules.
    pub async fn confirm_delivery(&self, a: &AuthorizedAccount, id: &str) -> Result<Submission> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE submissions SET state=state WHERE id=?").bind(id).execute(&mut *tx).await?;
        let raw: String = sqlx::query_scalar("SELECT document FROM submissions WHERE id=? AND state='delivery_unknown'").bind(id).fetch_optional(&mut *tx).await?.ok_or(Error::Conflict)?;
        let mut submission: Submission = serde_json::from_str(&raw)?;
        submission.state = SubmissionState::Accepted; submission.revision += 1;
        sqlx::query("UPDATE submissions SET state='accepted',document=? WHERE id=?").bind(serde_json::to_string(&submission)?).bind(id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'submission_changed',?)").bind(submission.revision).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(submission)
    }
    pub async fn needs_filing(&self, a: &AuthorizedAccount) -> Result<Vec<Submission>> {
        let db = self.db(a).await?;
        let rows:Vec<String>=sqlx::query_scalar("SELECT document FROM submissions WHERE state='accepted' AND filing_status IN ('pending','failed') ORDER BY due_at LIMIT 10").fetch_all(&db).await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub async fn set_filing(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        status: FilingStatus,
    ) -> Result<Submission> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE submissions SET state=state WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let raw: Option<String> =
            sqlx::query_scalar("SELECT document FROM submissions WHERE id=? AND state='accepted'")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let mut s: Submission = serde_json::from_str(&raw.ok_or(Error::Conflict)?)?;
        s.filing_status = status;
        s.revision += 1;
        let status = serde_json::to_value(&s.filing_status)?
            .as_str()
            .ok_or(Error::Invalid)?
            .to_owned();
        sqlx::query("UPDATE submissions SET filing_status=?,document=? WHERE id=?")
            .bind(status)
            .bind(serde_json::to_string(&s)?)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO events(revision,kind,target_id) VALUES(?,'submission_filing_changed',?)",
        )
        .bind(s.revision)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(s)
    }
}
