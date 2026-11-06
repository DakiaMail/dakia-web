use super::*;
impl Store {
    /// Only caller-verified Sent messages or deliberate composition choices qualify.
    /// Source keys make repeat scans idempotent rather than inflating contact scores.
    pub async fn learn_recipients(
        &self,
        a: &AuthorizedAccount,
        source: &str,
        addresses: &[Address],
        at: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        if source.is_empty() || addresses.len() > 1000 {
            return Err(Error::Invalid);
        }
        let at = DateTime::parse_from_rfc3339(at)
            .map_err(|_| Error::Invalid)?
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        for address in addresses {
            if address.email.len() > 320
                || !address.email.contains('@')
                || address.email.chars().any(char::is_control)
            {
                continue;
            }
            sqlx::query("INSERT INTO recipient_evidence(source_id,email,document,observed_at) VALUES(?,?,?,?) ON CONFLICT(source_id,email) DO UPDATE SET document=excluded.document,observed_at=excluded.observed_at")
                .bind(source).bind(address.email.trim()).bind(serde_json::to_string(address)?).bind(&at).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn recipient_suggestions(
        &self,
        a: &AuthorizedAccount,
        needle: &str,
    ) -> Result<Vec<Address>> {
        let db = self.db(a).await?;
        // Counts represent distinct evidence, not sync frequency. No incoming senders.
        let rows: Vec<String> = sqlx::query_scalar("SELECT document FROM recipient_evidence WHERE instr(lower(email),lower(?))>0 OR instr(lower(COALESCE(json_extract(document,'$.name'),'')),lower(?))>0 GROUP BY email ORDER BY COUNT(*) DESC,MAX(observed_at) DESC,email LIMIT 20")
            .bind(needle).bind(needle).fetch_all(&db).await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub async fn known_recipient(&self, a: &AuthorizedAccount, email: &str) -> Result<bool> {
        let db = self.db(a).await?;
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipient_evidence WHERE email=?)")
                .bind(email.trim())
                .fetch_one(&db)
                .await?,
        )
    }
}
