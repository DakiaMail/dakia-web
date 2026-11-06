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
