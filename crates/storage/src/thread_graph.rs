use super::*;
#[cfg(test)]
#[path = "thread_graph_tests.rs"]
mod tests;
impl Store {
    pub async fn thread_revision(&self, a: &AuthorizedAccount) -> Result<i64> {
        self.account_repository(a).await?.thread_revision().await
    }
    pub async fn thread_verified(
        &self,
        a: &AuthorizedAccount,
        component: &str,
        revision: i64,
    ) -> Result<bool> {
        self.account_repository(a)
            .await?
            .thread_verified(component, revision)
            .await
    }
    pub async fn verify_thread(&self, a: &AuthorizedAccount, component: &str) -> Result<()> {
        self.verify_thread_revision(a, component, self.thread_revision(a).await?)
            .await
            .map(|_| ())
    }
    pub async fn verify_thread_revision(
        &self,
        a: &AuthorizedAccount,
        component: &str,
        revision: i64,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a)
            .await?
            .verify_thread_revision(component, revision)
            .await
    }
    pub async fn thread_neighbors(
        &self,
        a: &AuthorizedAccount,
        edges: &[String],
    ) -> Result<Vec<(Message, String)>> {
        self.account_repository(a)
            .await?
            .thread_neighbors(edges)
            .await
    }
    pub async fn thread_coverage_fresh(&self, a: &AuthorizedAccount) -> Result<bool> {
        self.account_repository(a)
            .await?
            .thread_coverage_fresh()
            .await
    }
}
