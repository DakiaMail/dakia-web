use super::*;

impl Store {
    /// The Scheduled view is not an operation audit log. Read immutable send
    /// snapshots, retaining all historical records without mutating them.
    pub async fn scheduled_messages(&self, a: &AuthorizedAccount) -> Result<Vec<ScheduledMessage>> {
        let db = self.db(a).await?;
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT document, snapshot FROM submissions
             WHERE json_extract(request, '$.due_at') IS NOT NULL
             AND (state IN ('scheduled','claimed','submitting','failed','delivery_unknown')
                  OR (state='accepted' AND filing_status='failed'))
             ORDER BY due_at ASC",
        )
        .fetch_all(&db)
        .await?;
        rows.into_iter()
            .map(|(document, snapshot)| {
                let draft: Draft = serde_json::from_str(&snapshot)?;
                Ok(ScheduledMessage {
                    submission: serde_json::from_str(&document)?,
                    subject: draft.subject,
                    to: draft.to,
                })
            })
            .collect()
    }
    pub async fn drafts(&self, a: &AuthorizedAccount) -> Result<Vec<Draft>> {
        let db = self.db(a).await?;
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT document FROM drafts ORDER BY json_extract(document,'$.saved_at') DESC",
        )
        .fetch_all(&db)
        .await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub async fn submissions(&self, a: &AuthorizedAccount) -> Result<Vec<Submission>> {
        let db = self.db(a).await?;
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT document FROM submissions ORDER BY due_at DESC LIMIT 200")
                .fetch_all(&db)
                .await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
}
