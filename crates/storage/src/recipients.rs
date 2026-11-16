use super::*;
impl Store {
    /// Verified Sent mail and deliberate composition choices are the only inputs.
    pub async fn learn_recipients(
        &self,
        a: &AuthorizedAccount,
        source: &str,
        addresses: &[Address],
        at: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .learn_recipients(source, addresses, at)
            .await
    }
    pub async fn recipient_suggestions(
        &self,
        a: &AuthorizedAccount,
        needle: &str,
    ) -> Result<Vec<Address>> {
        self.account_repository(a)
            .await?
            .recipient_suggestions(needle)
            .await
    }
    pub async fn known_recipient(&self, a: &AuthorizedAccount, email: &str) -> Result<bool> {
        self.account_repository(a)
            .await?
            .known_recipient(email)
            .await
    }
}
