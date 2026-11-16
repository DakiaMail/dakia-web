use super::*;

pub(super) async fn status(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<IndexingStatus>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let excluded = app.store.search_excluded_folders(&a).await?;
    let folders = app
        .store
        .provider_document(&a, "sync:folders")
        .await?
        .and_then(|raw| serde_json::from_str::<crate::sync::State>(&raw).ok());
    let terminal = app.store.terminal_scan_folders(&a).await?;
    let discovery_complete = folders.as_ref().is_some_and(|state| {
        !state.folders.is_empty()
            && state
                .folders
                .iter()
                .filter(|f| {
                    f.selectable && !excluded.iter().any(|e| e.eq_ignore_ascii_case(&f.name))
                })
                .all(|f| terminal.contains(&f.name))
    });
    let (coverage, mut search_unavailable) = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        app.store.text_coverage(&a),
    )
    .await
    {
        Ok(Ok(coverage)) => (coverage, false),
        _ => (app.store.indexing_inventory(&a).await?, true),
    };
    let mut vector_pending = false;
    if coverage.semantic_required && coverage.eligible > 0 {
        if let Some(revision) = coverage.semantic_revision.as_deref() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                app.store.search_vectors_ready(&a, revision),
            )
            .await
            {
                Ok(Ok(ready)) => vector_pending = !ready,
                _ => search_unavailable = true,
            }
        }
    }
    let backlog = coverage
        .backlog
        .max(coverage.lexical_pending)
        .max(i64::from(vector_pending));
    let delivery_failed = app
        .store
        .provider_document(&a, "search:delivery_error")
        .await?
        .is_some_and(|value| !value.is_empty());
    let provider_ready = app
        .store
        .accounts(&user.user)
        .await?
        .iter()
        .any(|a| a.id == account && a.state == AccountState::Ready);
    let acquisition_wait = coverage.dirty == 0
        && coverage.delivery_pending == 0
        && coverage.semantic_pending <= coverage.pending + coverage.retrying;
    let provider_paused = !provider_ready
        && acquisition_wait
        && (coverage.semantic_pending > 0 || !discovery_complete);
    let stalled = discovery_complete
        && ((coverage.semantic_required && !coverage.semantic_available)
            || coverage.semantic_retrying > 0
            || backlog == 0
            || (coverage.pending == 0 && coverage.dirty == 0 && coverage.retrying > 0));
    let failed = search_unavailable
        || delivery_failed
        || coverage.lexical_failed
        || coverage.semantic_failed
        || coverage.semantic_retrying > 0
        || coverage.retrying > 0;
    let state = if !failed
        && discovery_complete
        && coverage.lexical_discovered
        && !coverage.lexical_failed
        && backlog == 0
        && coverage.processed >= coverage.eligible
    {
        IndexingState::Complete
    } else if (!app.store.jobs_enabled().await? && !vector_pending)
        || (!failed && (provider_paused || stalled))
    {
        IndexingState::Paused
    } else if failed {
        IndexingState::Error
    } else if !discovery_complete {
        IndexingState::Discovering
    } else {
        IndexingState::Indexing
    };
    Ok(Json(IndexingStatus {
        account_id: account,
        state,
        discovery_complete,
        eligible: coverage.eligible.min(u32::MAX as i64) as u32,
        processed: coverage.processed.max(0).min(u32::MAX as i64) as u32,
        backlog: backlog.min(u32::MAX as i64) as u32,
        gaps: (coverage.gaps - coverage.retrying)
            .max(0)
            .min(u32::MAX as i64) as u32,
    }))
}
