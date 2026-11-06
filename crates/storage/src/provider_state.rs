use super::*;
impl Store {
    pub async fn account_state(
        &self,
        a: &AuthorizedAccount,
        state: AccountState,
    ) -> Result<Account> {
        let _guard = self.gate.read().await;
        self.authorize(&a.user_id, &a.id).await?;
        let mut tx = self.control.begin().await?;
        sqlx::query("UPDATE accounts SET metadata=metadata WHERE id=? AND user_id=?")
            .bind(&a.id)
            .bind(&a.user_id)
            .execute(&mut *tx)
            .await?;
        let raw: String =
            sqlx::query_scalar("SELECT metadata FROM accounts WHERE id=? AND user_id=?")
                .bind(&a.id)
                .bind(&a.user_id)
                .fetch_one(&mut *tx)
                .await?;
        let mut account: Account = serde_json::from_str(&raw)?;
        account.state = state;
        account.revision += 1;
        sqlx::query("UPDATE accounts SET metadata=? WHERE id=? AND user_id=?")
            .bind(serde_json::to_string(&account)?)
            .bind(&a.id)
            .bind(&a.user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(account)
    }
    pub async fn put_provider_document(
        &self,
        a: &AuthorizedAccount,
        key: &str,
        document: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE document<>excluded.document").bind(key).bind(document).execute(&db).await?;
        Ok(())
    }
    /// Atomically bind unsubscribe intent to an immutable normal SMTP submission.
    /// No editable draft exists. A crash cannot separate the claim and the outbox.
    pub async fn enqueue_unsubscribe(
        &self,
        a: &AuthorizedAccount,
        key: &str,
        draft: Draft,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        if draft.account_id != a.id
            || draft.to.len() != 1
            || !draft.cc.is_empty()
            || !draft.bcc.is_empty()
            || !draft.attachments.is_empty()
            || draft.in_reply_to.is_some()
            || !draft.references.is_empty()
        {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let id = Uuid::new_v4().to_string();
        let record = format!("submission:{id}");
        let inserted = sqlx::query(
            "INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO NOTHING",
        )
        .bind(key)
        .bind(&record)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            return Ok(());
        }
        let s = Submission {
            id,
            account_id: a.id.clone(),
            operation_id: key.into(),
            draft_id: draft.id.clone(),
            draft_revision: draft.revision,
            state: SubmissionState::Scheduled,
            due_at: now(),
            timezone: "UTC".into(),
            filing_status: FilingStatus::Pending,
            revision: 1,
        };
        sqlx::query("INSERT INTO submissions(id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES(?,?,?,?,?,?,?,'scheduled',?,'pending')")
            .bind(&s.id).bind(key).bind(&record).bind(&draft.id).bind(draft.revision)
            .bind(serde_json::to_string(&draft)?).bind(serde_json::to_string(&s)?).bind(&s.due_at)
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO outbox(kind,target_id) VALUES('submit',?)")
            .bind(&s.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO events(revision,kind,target_id) VALUES(1,'submission_scheduled',?)",
        )
        .bind(&s.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
    /// Atomic account-local claim for a non-repeatable external unsubscribe POST.
    pub async fn claim_unsubscribe(&self, a: &AuthorizedAccount, key: &str) -> Result<bool> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        Ok(sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?, 'attempted') ON CONFLICT(key) DO NOTHING")
            .bind(key).execute(&db).await?.rows_affected() == 1)
    }
    /// Exact-prefix, account-local lookup for previously scanned Gmail memberships.
    /// Bound work on a stale reference; this is not a global metadata enumeration.
    pub async fn message_reference_documents(
        &self,
        a: &AuthorizedAccount,
        message_id: &str,
    ) -> Result<Vec<String>> {
        let db = self.db(a).await?;
        let prefix = format!("member-reference:{message_id}:");
        let upper = format!("member-reference:{message_id};");
        Ok(sqlx::query_scalar(
            "SELECT document FROM provider_documents WHERE key>=? AND key<? ORDER BY key LIMIT 256",
        )
        .bind(&prefix)
        .bind(&upper)
        .fetch_all(&db)
        .await?)
    }
    pub async fn provider_document(
        &self,
        a: &AuthorizedAccount,
        key: &str,
    ) -> Result<Option<String>> {
        let db = self.db(a).await?;
        Ok(
            sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=?")
                .bind(key)
                .fetch_optional(&db)
                .await?,
        )
    }
}
