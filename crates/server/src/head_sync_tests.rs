use super::*;

#[tokio::test]
async fn routine_polling_yields_to_idle_hints_and_explicit_refresh_without_replaying_history() {
    let (_dir, app, user, a, provider) = paged(true).await;
    provider.heads.store(true, Ordering::SeqCst);
    assert!(app.mail.check_head(&app.store, &a, "INBOX").await.unwrap());
    let ten_seconds_ago = (chrono::Utc::now().timestamp() - 10).to_string();
    app.store
        .put_provider_document(&a, "sync:head-at:INBOX", &ten_seconds_ago)
        .await
        .unwrap();
    for _ in 0..20 {
        assert!(!app.mail.poll_head(&app.store, &a, "INBOX").await.unwrap());
        assert!(!app.mail.sync_active(&a).await);
    }
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 1);
    // IDLE and manual refresh retain the short debounce, even while ordinary
    // polling is cooling down. They do not reset the historical continuation.
    let history = app.store.scan_state(&a, "INBOX").await.unwrap();
    search(
        &app,
        &user,
        serde_json::json!({"q":"folder:INBOX", "accounts":a.id(), "refresh":true}),
    )
    .await
    .unwrap();
    wait_refresh(&app, &a).await;
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 2);
    assert_eq!(app.store.scan_state(&a, "INBOX").await.unwrap(), history);
    app.store
        .put_provider_document(&a, "sync:head-at:INBOX", "0")
        .await
        .unwrap();
    assert!(app.mail.poll_head(&app.store, &a, "INBOX").await.unwrap());
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn sync_activity_is_account_local_transient_and_not_coverage() {
    let (_dir, app, user, a, provider) = paged(true).await;
    let other = app
        .store
        .accounts(&user)
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.id != a.id())
        .unwrap();
    let other = app.store.authorize(&user, &other.id).await.unwrap();
    assert!(!app.mail.sync_active(&a).await);
    assert!(!app.mail.sync_active(&other).await);
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), 0);
    let lane = app.mail.sync_lane(&a).await;
    let idle_guard = lane.lock().await;
    assert!(
        !app.mail.sync_active(&a).await,
        "scheduler bookkeeping is not provider activity"
    );
    drop(idle_guard);
    let guard = app.mail.begin_sync(&a).await;
    assert!(app.mail.sync_active(&a).await);
    assert!(!app.mail.sync_active(&other).await);
    drop(guard);
    assert!(!app.mail.sync_active(&a).await);
    // Real failed provider turn releases activity despite unfinished coverage.
    provider.base.fail.store(true, Ordering::SeqCst);
    crate::sync::tick(&app, &a).await.unwrap();
    assert!(!app.mail.sync_active(&a).await);
    assert_ne!(
        app.store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Complete
    );
    let runtime = app.mail.clone();
    let account = a.clone();
    let task = tokio::spawn(async move {
        let _guard = runtime.begin_sync(&account).await;
        std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;
    assert!(app.mail.sync_active(&a).await);
    task.abort();
    let _ = task.await;
    assert!(!app.mail.sync_active(&a).await);
}

#[tokio::test]
async fn idle_supervisor_drives_durable_heads_and_stops_when_account_is_revoked() {
    let (_dir, app, _, a, provider) = paged(true).await;
    provider.heads.store(true, Ordering::SeqCst);
    let listener = tokio::spawn(crate::idle::run(app.clone()));
    tokio::time::timeout(Duration::from_secs(15), async {
        while provider.head_calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
    assert!(
        app.store.next_push(&a).await.unwrap().is_none(),
        "hints alone and bootstrap never notify"
    );
    assert!(app
        .store
        .provider_document(&a, "sync:head:INBOX")
        .await
        .unwrap()
        .is_some());
    app.store
        .account_state(&a, dakia_domain::AccountState::Reconnect)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let before = provider.head_calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), before);
    listener.abort();
    let _ = listener.await;
}

#[tokio::test]
async fn independent_heads_arrive_during_history_and_continue_after_limited_cooldown_without_pruning(
) {
    let (_dir, app, user, a, provider) = paged(true).await;
    provider.heads.store(true, Ordering::SeqCst);
    crate::sync::tick(&app, &a).await.unwrap(); // historical INBOX
    let before = app.store.scan_state(&a, "INBOX").await.unwrap();
    crate::sync::tick(&app, &a).await.unwrap(); // Archive
    crate::sync::tick(&app, &a).await.unwrap(); // independent INBOX head
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 1);
    assert_eq!(app.store.scan_state(&a, "INBOX").await.unwrap(), before);
    assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
    let req = serde_json::json!({"q":"folder:INBOX","accounts":a.id()});
    let result = search(&app, &user, req).await.unwrap();
    assert_eq!(result.rows.len(), 1);
    wait_refresh(&app, &a).await;
    // Force only the test clock budget due, not provider or snapshot state.
    for _ in 0..30 {
        app.store
            .put_provider_document(&a, "sync:head-at:INBOX", "0")
            .await
            .unwrap();
        crate::sync::tick(&app, &a).await.unwrap();
    }
    let visits = provider.visited.lock().await.clone();
    for folder in ["INBOX", "Archive", "Sent"] {
        assert_eq!(
            visits.iter().filter(|(f, _)| f == folder).count(),
            3,
            "{folder}"
        );
        assert!(app.store.scan_state(&a, folder).await.unwrap().is_none());
    }
    assert!(
        provider.head_calls.load(Ordering::SeqCst) > 3,
        "head checks continue while historical scan cools down"
    );
    assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
    assert_eq!(
        app.store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Partial
    );
    assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn request_heads_serialize_preserve_history_and_failures_never_advance_checkpoint() {
    let (_dir, app, user, a, provider) = paged(true).await;
    provider.heads.store(true, Ordering::SeqCst);
    crate::sync::tick(&app, &a).await.unwrap();
    let before = app.store.scan_state(&a, "INBOX").await.unwrap();
    let req = serde_json::json!({"q":"folder:INBOX","accounts":a.id()});
    let left = search(&app, &user, req.clone());
    let right = search(&app, &user, req.clone());
    let (l, r) = tokio::join!(left, right);
    assert!(l.is_ok() && r.is_ok());
    wait_refresh(&app, &a).await;
    assert_eq!(app.store.scan_state(&a, "INBOX").await.unwrap(), before);
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 1);
    let checkpoint = app
        .store
        .provider_document(&a, "sync:head:INBOX")
        .await
        .unwrap();
    provider.base.fail.store(true, Ordering::SeqCst);
    app.store
        .put_provider_document(&a, "sync:head-at:INBOX", "0")
        .await
        .unwrap();
    let lane = app.mail.sync_lane(&a).await;
    let _guard = lane.lock().await;
    assert!(app.mail.check_head(&app.store, &a, "INBOX").await.is_err());
    assert_eq!(
        app.store
            .provider_document(&a, "sync:head:INBOX")
            .await
            .unwrap(),
        checkpoint
    );
    assert_eq!(app.store.scan_state(&a, "INBOX").await.unwrap(), before);
    assert!(
        !app.mail.check_head(&app.store, &a, "INBOX").await.unwrap(),
        "failure also consumes polling budget"
    );
    let accounts = app.store.accounts(&user).await.unwrap();
    let other = accounts.iter().find(|b| b.id != a.id()).unwrap();
    let other = app.store.authorize(&user, &other.id).await.unwrap();
    assert!(app
        .store
        .provider_document(&other, "sync:head:INBOX")
        .await
        .unwrap()
        .is_none());
    assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn historical_200_page_is_ingested_end_to_end_without_changing_ui_page_size() {
    let (_dir, app, _, a, _) = setup(200, 0).await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 200);
    assert_eq!(app.store.catalogue(&a).await.unwrap().len(), 200);
}
