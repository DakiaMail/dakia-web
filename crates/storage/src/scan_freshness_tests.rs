use super::*;

#[tokio::test]
async fn limited_scan_freshness_expiry_conflicts_errors_and_pruning_are_independent() {
    let (_dir, store, _, a, _) = fixture().await;
    store
        .upsert_message(&a, message(&a, "preserve"))
        .await
        .unwrap();
    store.seed_inbox_memberships(&a).await.unwrap();
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let mut page = SearchPage {
        rows: vec![],
        continuation: Some("next".into()),
        coverage: vec![AccountCoverage {
            account_id: a.id().into(),
            folder_ids: vec!["INBOX".into()],
            state: CoverageState::Partial,
            error: None,
            inbox: None,
        }],
        classification: ClassificationCoverage {
            classified: 0,
            total: 0,
        },
        pending: false,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
    };
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    let initial = store.inbox_coverage(&a).await.unwrap().inbox.unwrap();
    assert!(initial.refreshing && initial.last_scanned_at.is_none());
    page.continuation = None;
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    let terminal = store.inbox_coverage(&a).await.unwrap();
    assert_eq!(terminal.state, CoverageState::Partial);
    assert!(!terminal.inbox.as_ref().unwrap().refreshing);
    assert!(terminal.inbox.as_ref().unwrap().last_scanned_at.is_some());
    // Replayed terminal commits cannot extend cooldown or prune retained rows.
    assert!(matches!(
        store.record_scan(&a, "INBOX", generation, &page).await,
        Err(Error::Conflict)
    ));
    let db = store.db(&a).await.unwrap();
    let completed: Option<i64> =
        sqlx::query_scalar("SELECT completed_at FROM folder_scans WHERE folder='INBOX'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(completed, None);
    assert_eq!(store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
    store
        .inbox_refresh_result(&a, Some("provider_unavailable"))
        .await
        .unwrap();
    let error = store.inbox_coverage(&a).await.unwrap();
    assert_eq!(error.state, CoverageState::Error);
    assert_eq!(error.inbox, terminal.inbox);
    store.inbox_refresh_result(&a, None).await.unwrap();
    sqlx::query("UPDATE folder_scans SET last_scanned_at=last_scanned_at-301 WHERE folder='INBOX'")
        .execute(&db)
        .await
        .unwrap();
    let (left, right) = tokio::join!(store.scan_state(&a, "INBOX"), store.scan_state(&a, "INBOX"));
    let next = left.unwrap().unwrap();
    assert_eq!(right.unwrap().unwrap(), next);
    assert_eq!(next.0, generation + 1);
    assert_eq!(next.1, None);
    let routine = store.inbox_coverage(&a).await.unwrap().inbox.unwrap();
    assert!(routine.refreshing && routine.last_scanned_at.is_some());
    assert!(matches!(
        store.record_scan(&a, "INBOX", generation, &page).await,
        Err(Error::Conflict)
    ));
    // A later genuinely complete generation may reconcile; cooldown still holds.
    page.coverage[0].state = CoverageState::Complete;
    store.record_scan(&a, "INBOX", next.0, &page).await.unwrap();
    assert_eq!(
        store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Complete
    );
    assert!(store.inbox_rows(&a, None, None).await.unwrap().is_empty());
    assert!(store.scan_state(&a, "INBOX").await.unwrap().is_none());
}
