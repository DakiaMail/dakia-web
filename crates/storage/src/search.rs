use super::*;
#[derive(Default, Debug)]
pub struct CachedSearchFilter<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub text: Option<&'a str>,
    pub text_units: Option<&'a [String]>,
    pub text_alternatives: Option<&'a [Vec<String>]>,
    /// Inclusive/exclusive ISO calendar days from the provider INTERNALDATE.
    pub since: Option<&'a str>,
    pub before: Option<&'a str>,
    pub seen: Option<bool>,
    pub starred: Option<bool>,
    pub folders: Option<&'a [String]>,
    pub classification: ClassificationFilter,
    pub category: Option<&'a str>,
    pub snoozed_only: bool,
    pub hide_snoozed: bool,
}
impl CachedSearchFilter<'_> {
    pub(crate) fn matches_date(&self, message: &Message) -> bool {
        if self.since.is_none() && self.before.is_none() {
            return true;
        }
        message.received_date.as_deref().is_some_and(|day| {
            if chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_or(true, |date| date.format("%Y-%m-%d").to_string() != day)
            {
                return false;
            }
            self.since.is_none_or(|since| day >= since)
                && self.before.is_none_or(|before| day < before)
        })
    }
}
impl Store {
    pub async fn search_dates_known(&self, a: &AuthorizedAccount) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        match &self.search_database {
            Some(db) => db.dates_known(a.id()).await,
            None => Ok(false),
        }
    }
    /// Service maintenance of derived, local vector graphs. No message content
    /// or candidate identities leave the selected search repository.
    pub async fn maintain_search_vectors(&self, snapshot: bool) -> Result<bool> {
        let _guard = self.gate.read().await;
        match &self.search_database {
            Some(db) => db.maintain_vectors(snapshot).await,
            None => Ok(false),
        }
    }
    pub async fn search_vectors_ready(
        &self,
        a: &AuthorizedAccount,
        revision: &str,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        match &self.search_database {
            Some(db) => db.vectors_ready(a.id(), revision).await,
            None => Ok(false),
        }
    }
    /// Search all cached metadata, returning a bounded positive preview, never
    /// evidence of absence. No bodies, snippets or provider calls are involved.
    pub async fn cached_search_rows(
        &self,
        a: &AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
    ) -> Result<Vec<Message>> {
        self.account_repository(a)
            .await?
            .cached_search_rows(filter)
            .await
    }

    /// Provisional header positives from at most 1,024 recent messages. This
    /// budget never limits authoritative indexed search or provider search.
    pub async fn cached_search_preview_rows(
        &self,
        a: &AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
    ) -> Result<Vec<Message>> {
        let mut timing = crate::timing::SearchTiming::new("header_preview_authorization");
        let repository = self.account_repository(a).await?;
        timing.stage("header_preview_query");
        repository.cached_search_preview_rows(filter).await
    }
    /// Opaque, owner-bound continuation. Mailbox state never enters URLs.
    pub async fn save_search_cursor(&self, user: &str, document: &str) -> Result<String> {
        let _guard = self.gate.read().await;
        let id = Uuid::new_v4().to_string();
        self.control_repository()
            .save_cursor(
                user,
                &id,
                document,
                Utc::now().timestamp(),
                Utc::now().timestamp() + 3600,
            )
            .await?;
        Ok(id)
    }
    pub async fn load_search_cursor(&self, user: &str, id: &str) -> Result<String> {
        Uuid::parse_str(id).map_err(|_| Error::CursorExpired)?;
        let raw = self
            .control_repository()
            .cursor(user, id, Utc::now().timestamp())
            .await?;
        raw.ok_or(Error::CursorExpired)
    }
}
