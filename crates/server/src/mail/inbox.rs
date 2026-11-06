//! Local inbox reads never wait on a provider or its reconciliation lane.
use super::*;
use std::time::Instant;
use tracing::Instrument;

impl MailRuntime {
    /// Caller owns sync_lane. A head has its own persisted watermark and poll
    /// budget, independent of the historical continuation and its cooldown.
    pub(crate) async fn check_head(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<bool> {
        self.check_head_due(store, a, folder, 5).await
    }

    pub(crate) async fn poll_head(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<bool> {
        self.check_head_due(store, a, folder, 60).await
    }

    async fn check_head_due(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        interval: i64,
    ) -> Result<bool> {
        let provider = self.provider(store, a).await?;
        if !provider.supports_head() {
            return Ok(false);
        }
        let at_key = format!("sync:head-at:{folder}");
        let now = chrono::Utc::now().timestamp();
        let at = store
            .provider_document(a, &at_key)
            .await?
            .and_then(|s| s.parse::<i64>().ok());
        if at.is_some_and(|at| now.saturating_sub(at) < interval) {
            return Ok(false);
        }
        // Failed attempts also consume a polling budget, but never a watermark.
        store
            .put_provider_document(a, &at_key, &now.to_string())
            .await?;
        let after: Option<p::HeadCheckpoint> = store
            .provider_document(a, &format!("sync:head:{folder}"))
            .await?
            .and_then(|s| serde_json::from_str(&s).ok());
        let lane = self.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        let _activity = self.begin_sync(a).await;
        let page = provider
            .head(folder, after.as_ref())
            .await
            .map_err(provider_error)?;
        if page.checkpoint.account_id != a.id() || page.checkpoint.folder != folder {
            return Err(ApiFailure(StatusCode::FORBIDDEN, "account_mismatch"));
        }
        if page.rows.len() > 50
            || after.as_ref().is_some_and(|old| {
                old.uidvalidity == page.checkpoint.uidvalidity && page.checkpoint.uid < old.uid
            })
            || page
                .rows
                .iter()
                .any(|m| m.reference.uid > page.checkpoint.uid)
        {
            return Err(ApiFailure(
                StatusCode::BAD_GATEWAY,
                "invalid_provider_metadata",
            ));
        }
        let rows = self
            .ingest_metadata(store, a, folder, page.checkpoint.uidvalidity, page.rows)
            .await?;
        store
            .record_incremental(
                a,
                folder,
                &rows,
                &serde_json::to_string(&page.checkpoint).map_err(Error::from)?,
            )
            .await?;
        Ok(true)
    }

    pub(crate) async fn sync_lane(&self, a: &AuthorizedAccount) -> Arc<Mutex<()>> {
        self.sync_lanes
            .lock()
            .await
            .entry(a.id().into())
            .or_default()
            .clone()
    }

    pub(crate) async fn begin_sync(&self, a: &AuthorizedAccount) -> SyncActivity {
        let counter = self
            .sync_activity
            .lock()
            .await
            .entry(a.id().into())
            .or_default()
            .clone();
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        SyncActivity(counter)
    }

    /// Only actual provider refresh work, not scheduler checks or lock waits.
    pub(crate) async fn sync_active(&self, a: &AuthorizedAccount) -> bool {
        self.sync_activity
            .lock()
            .await
            .get(a.id())
            .is_some_and(|counter| counter.load(std::sync::atomic::Ordering::Relaxed) > 0)
    }

    /// At most one queued/running refresh per account. One page per turn keeps
    /// ordinary background backfill and mutations moving; cooldown bounds polls.
    pub(crate) async fn refresh_inbox(&self, store: &Store, a: &AuthorizedAccount, explicit: bool) {
        let slot = self
            .inbox_refreshes
            .lock()
            .await
            .entry(a.id().into())
            .or_default()
            .clone();
        let Ok(mut guard) = slot.try_lock_owned() else {
            return;
        };
        if guard.is_some_and(|t| t.elapsed() < Duration::from_secs(if explicit { 5 } else { 60 })) {
            return;
        }
        *guard = Some(Instant::now());
        let (runtime, store, a) = (self.clone(), store.clone(), a.clone());
        let span = tracing::info_span!(target: "dakia_server::diagnostics", "inbox_refresh",
            account_key = crate::diagnostics::correlation(a.id()),
            refresh_key = %uuid::Uuid::new_v4());
        tokio::spawn(async move {
            let started = Instant::now();
            tracing::info!(target: "dakia_server::diagnostics", stage = "queued", "inbox refresh started");
            let lane = runtime.sync_lane(&a).await;
            let _sync = lane.lock().await;
            let lane_wait_ms = started.elapsed().as_millis() as u64;
            let mut stage = "seed_memberships";
            let result = async {
                store.seed_inbox_memberships(&a).await?;
                stage = "head";
                let checked = if explicit {
                    runtime.check_head(&store, &a, "INBOX").await?
                } else {
                    runtime.poll_head(&store, &a, "INBOX").await?
                };
                if checked {
                    stage = "persist_head";
                    store.inbox_refresh_result(&a, None).await?;
                    return Ok(());
                }
                stage = "scan_state";
                let (scan, verified) = runtime.prepare_scan(&store, &a, "INBOX").await?;
                if verified {
                    stage = "unchanged_snapshot";
                    store.inbox_refresh_result(&a, None).await?;
                    return Ok(());
                }
                let Some((generation, continuation)) = scan else {
                    stage = "no_scan_state";
                    return Ok(());
                };
                stage = "scan_page";
                runtime
                    .record_snapshot_page(&store, &a, "INBOX", generation, continuation.as_deref())
                    .await?;
                store.inbox_refresh_result(&a, None).await?;
                Ok::<_, ApiFailure>(())
            }
            .await;
            if let Err(error) = result {
                tracing::warn!(target: "dakia_server::diagnostics", stage, reason = error.1,
                    lane_wait_ms, elapsed_ms = started.elapsed().as_millis() as u64,
                    outcome = "failed", "inbox refresh completed; saved mail retained");
                if error.1 == "provider_reference_changed" {
                    let _ = store.reset_scan(&a, "INBOX").await;
                }
                // Store only the API's allowlisted code, never provider text.
                if store.inbox_refresh_result(&a, Some(error.1)).await.is_err() {
                    tracing::warn!(target: "dakia_server::diagnostics", stage = "persist_error",
                        "inbox refresh error not persisted");
                }
            } else {
                tracing::info!(target: "dakia_server::diagnostics", stage,
                    lane_wait_ms, elapsed_ms = started.elapsed().as_millis() as u64,
                    outcome = if stage == "no_scan_state" { "skipped" } else { "success" }, "inbox refresh completed");
            }
            *guard = Some(Instant::now());
        }.instrument(span));
    }
}
