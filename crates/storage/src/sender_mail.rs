use super::*;
impl Store {
    /// A local count and keyset page share one snapshot. Neither proves provider coverage.
    pub async fn cached_sender_mail(
        &self,
        a: &AuthorizedAccount,
        sender: &str,
        cutoff: &str,
        after: Option<&str>,
        trash: &[String],
    ) -> Result<(i64, Vec<Message>)> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .cached_sender_mail(sender, cutoff, after, trash)
            .await
    }
}
