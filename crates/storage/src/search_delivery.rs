//! At-least-once source-guarded delivery between independent databases.
use super::*;
use crate::search_backend::SearchConnection;

impl Store {
    /// Explicit configuration keeps tests and offline SQLite operators independent
    /// of process environment. Missing configuration uses the local index path.
    /// Configure at startup, before sharing this Store with request handlers/workers.
    pub async fn configure_search_database(&mut self, url: Option<&str>) -> Result<()> {
        let repository = Arc::new(SearchConnection::new(
            &self.root,
            url.or(self.default_search_url.as_deref()),
        )?);
        self.search_database = Some(repository.clone());
        self.account_pools.lock().await.clear();
        self.search_cleanup_done
            .store(false, std::sync::atomic::Ordering::Release);
        let control = self.control_repository();
        let startup = tokio::spawn(async move {
            let mut delay = 1;
            while repository.warm().await.is_err() {
                tracing::warn!(
                    operation = "search_index",
                    outcome = "unavailable",
                    "search database will reconnect in the background"
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                delay = (delay * 2).min(30);
            }
            if let Ok(removed) = control.removed_accounts().await {
                for id in removed {
                    if repository.remove_account(&id).await.is_err() {
                        break;
                    }
                }
            }
        });
        self.search_warm = Some(Arc::new(crate::search_backend::SearchWarm(
            startup.abort_handle(),
        )));
        Ok(())
    }

    pub async fn flush_search_index(&self, a: &AuthorizedAccount) -> Result<bool> {
        let Some(target) = &self.search_database else {
            return Ok(false);
        };
        let _guard = self.gate.read().await;
        let repository = self.account_repository(a).await?;
        let identity =
            tokio::time::timeout(std::time::Duration::from_millis(250), target.identity())
                .await
                .map_err(|_| Error::Invalid)??;
        if !self
            .search_cleanup_done
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.cleanup_removed_accounts().await?;
            self.search_cleanup_done
                .store(true, std::sync::atomic::Ordering::Release);
        }
        let current = repository.provider_document("search:database").await?;
        if current.as_deref() != Some(&identity) {
            target.remove_account(a.id()).await?;
            repository.requeue_search_index(&identity).await?;
        }
        repository.seed_search_versions().await?;
        let records = repository.search_deliveries().await?;
        if records.is_empty() {
            return Ok(false);
        }
        target.deliver_batch(a.id(), &records).await?;
        repository.acknowledge_search_deliveries(&records).await?;
        Ok(true)
    }

    pub(crate) async fn independent_search_rows(
        &self,
        a: &AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
        before: Option<(&str, &str)>,
    ) -> Result<Vec<Message>> {
        let repository = self.account_repository(a).await?;
        let target = self.search_database.as_ref().ok_or(Error::Invalid)?;
        let excluded = self.search_excluded_folders(a).await?;
        let mut result = Vec::new();
        let mut after = before.map(|(date, id)| (date.to_owned(), id.to_owned()));
        // Refill stale positives instead of allowing one bad record to hide later matches.
        loop {
            let candidates = target
                .rows(
                    a.id(),
                    filter,
                    after
                        .as_ref()
                        .map(|(date, id)| (date.as_str(), id.as_str())),
                )
                .await?;
            let count = candidates.len();
            let ids: Vec<_> = candidates.iter().map(|row| row.0.id.clone()).collect();
            let mut current_rows: std::collections::HashMap<_, _> = repository
                .current_search_messages(&ids)
                .await?
                .into_iter()
                .map(|(message, snoozed)| (message.id.clone(), (message, snoozed)))
                .collect();
            if count == 0 {
                break;
            }
            for (indexed, source) in candidates {
                after = Some((indexed.received_at.clone(), indexed.id.clone()));
                let Some((current, snoozed)) = current_rows.remove(&indexed.id) else {
                    continue;
                };
                if current.provider_id != source
                    || current.folder_ids.is_empty()
                    || current
                        .folder_ids
                        .iter()
                        .any(|f| excluded.iter().any(|e| e.eq_ignore_ascii_case(f)))
                {
                    continue;
                }
                if !filter.classification.matches(&current, &now())
                    || !filter.matches_date(&current)
                    || filter.seen.is_some_and(|seen| current.unread == seen)
                    || filter
                        .starred
                        .is_some_and(|starred| current.starred != starred)
                    || filter.folders.is_some_and(|folders| {
                        !current.folder_ids.iter().any(|f| folders.contains(f))
                    })
                    || filter
                        .category
                        .is_some_and(|c| current.category.as_ref().is_some_and(|value| value != c))
                {
                    continue;
                }
                if (filter.snoozed_only && !snoozed) || (filter.hide_snoozed && snoozed) {
                    continue;
                }
                result.push(current);
                if result.len() == 51 {
                    return Ok(result);
                }
            }
            if count < 51 {
                break;
            }
        }
        Ok(result)
    }
}

impl Store {
    pub async fn full_text_ranked_rows(
        &self,
        a: &AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
        after: Option<(f64, &str, &str)>,
    ) -> Result<Vec<(Message, f64)>> {
        self.full_text_ranked_candidates(a, filter, after, 51).await
    }
    pub async fn full_text_ranked_candidates(
        &self,
        a: &AuthorizedAccount,
        filter: &CachedSearchFilter<'_>,
        after: Option<(f64, &str, &str)>,
        limit: usize,
    ) -> Result<Vec<(Message, f64)>> {
        if !(1..=1025).contains(&limit) {
            return Err(Error::Invalid);
        }
        let mut timing = crate::timing::SearchTiming::new("lexical_authorization");
        let repository = self.account_repository(a).await?;
        let Some(target) = &self.search_database else {
            return Ok(self
                .full_text_rows(a, filter, after.map(|(_, d, i)| (d, i)))
                .await?
                .into_iter()
                .map(|m| (m, 0.0))
                .collect());
        };
        let excluded = self.search_excluded_folders(a).await?;
        let mut result = Vec::new();
        let mut cursor = after.map(|(s, d, i)| (s, d.to_owned(), i.to_owned()));
        loop {
            timing.stage("lexical_target");
            let candidates = target
                .ranked_candidates(
                    a.id(),
                    filter,
                    cursor
                        .as_ref()
                        .map(|(s, d, i)| (*s, d.as_str(), i.as_str())),
                    limit as i64,
                )
                .await?;
            timing.stage("lexical_source_validation");
            let count = candidates.len();
            let ids: Vec<_> = candidates.iter().map(|row| row.0.id.clone()).collect();
            let mut current_rows = std::collections::HashMap::new();
            for ids in ids.chunks(200) {
                for (message, snoozed) in repository.current_search_messages(ids).await? {
                    current_rows.insert(message.id.clone(), (message, snoozed));
                }
            }
            timing.stage("lexical_current_filters");
            for (indexed, source, score) in candidates {
                cursor = Some((score, indexed.received_at.clone(), indexed.id.clone()));
                let Some((current, snoozed)) = current_rows.remove(&indexed.id) else {
                    continue;
                };
                if current.provider_id != source
                    || current.folder_ids.is_empty()
                    || current
                        .folder_ids
                        .iter()
                        .any(|f| excluded.iter().any(|e| e.eq_ignore_ascii_case(f)))
                {
                    continue;
                }
                if !filter.classification.matches(&current, &now())
                    || !filter.matches_date(&current)
                    || filter.seen.is_some_and(|seen| current.unread == seen)
                    || filter
                        .starred
                        .is_some_and(|starred| current.starred != starred)
                    || filter.folders.is_some_and(|folders| {
                        !current.folder_ids.iter().any(|f| folders.contains(f))
                    })
                    || filter
                        .category
                        .is_some_and(|c| current.category.as_ref().is_some_and(|value| value != c))
                {
                    continue;
                }
                if (filter.snoozed_only && !snoozed) || (filter.hide_snoozed && snoozed) {
                    continue;
                }
                result.push((current, score));
                if result.len() == limit {
                    return Ok(result);
                }
            }
            if count < limit {
                break;
            }
        }
        Ok(result)
    }
}
