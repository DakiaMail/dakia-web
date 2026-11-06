//! Real executor/storage with disposable synthetic provider pages, no remote mail.
use super::*;
use crate::App;

async fn ready() -> (
    tempfile::TempDir,
    App,
    String,
    dakia_storage::AuthorizedAccount,
    Arc<PagedProvider>,
) {
    let (dir, app, user, a, provider) = paged(true).await;
    for other in app.store.accounts(&user).await.unwrap() {
        if other.id != a.id() {
            let scope = app.store.authorize(&user, &other.id).await.unwrap();
            app.store
                .account_state(&scope, AccountState::Reconnect)
                .await
                .unwrap();
        }
    }
    (dir, app, user, a, provider)
}
async fn wait_pages(p: &PagedProvider, n: usize, seconds: u64) {
    tokio::time::timeout(Duration::from_secs(seconds), async {
        while p.visited.lock().await.len() < n {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn executor_refills_bounded_turns_without_accelerating_mutation_maintenance() {
    let (_dir, app, _, a, provider) = ready().await;
    let job = tokio::spawn(crate::executor::run(app.clone()));
    wait_pages(&provider, 4, 3).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        provider.visited.lock().await.len(),
        4,
        "four jobs per admission window"
    );
    assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
    assert_eq!(
        app.store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Pending
    );
    let visits = provider.visited.lock().await.clone();
    assert_eq!(
        visits,
        vec![
            ("INBOX".into(), 0),
            ("Archive".into(), 0),
            ("INBOX".into(), 1),
            ("Sent".into(), 0)
        ]
    );
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    let before = provider.visited.lock().await.len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        provider.visited.lock().await.len(),
        before,
        "JoinSet dropped on shutdown"
    );
}

#[tokio::test]
async fn executor_failure_stops_burst_and_preserves_cursor_then_next_window_recovers() {
    let (_dir, app, _, a, provider) = ready().await;
    provider.base.fail.store(true, Ordering::SeqCst);
    let job = tokio::spawn(crate::executor::run(app.clone()));
    wait_pages(&provider, 1, 3).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        provider.visited.lock().await.len(),
        1,
        "failure cannot tight-loop"
    );
    assert!(app
        .store
        .inbox_coverage(&a)
        .await
        .unwrap()
        .inbox
        .unwrap()
        .last_scanned_at
        .is_none());
    provider.base.fail.store(false, Ordering::SeqCst);
    wait_pages(&provider, 5, 7).await;
    job.abort();
    let _ = job.await;
    assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn executor_idle_and_disabled_jobs_do_not_spin_or_reopen_limited_coverage() {
    let (_dir, app, _, a, provider) = ready().await;
    let job = tokio::spawn(crate::executor::run(app.clone()));
    wait_pages(&provider, 9, 14).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(app.store.scan_state(&a, "INBOX").await.unwrap().is_none());
    let before = provider.visited.lock().await.len();
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(provider.visited.lock().await.len(), before);
    assert_eq!(before, 9);
    job.abort();
    let _ = job.await;
    // A restored/disabled store must not start any provider metadata work.
    let restored = tempfile::tempdir().unwrap();
    let backup = tempfile::tempdir().unwrap();
    app.store
        .backup(backup.path().join("snapshot"))
        .await
        .unwrap();
    let store = Store::restore(
        backup.path().join("snapshot"),
        restored.path().join("restored"),
        [29; 32],
    )
    .await
    .unwrap();
    assert!(!store.jobs_enabled().await.unwrap());
    let mut disabled = app.clone();
    disabled.store = store;
    let disabled_job = tokio::spawn(crate::executor::run(disabled));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(provider.visited.lock().await.len(), before);
    disabled_job.abort();
    let _ = disabled_job.await;
}

#[tokio::test]
async fn executor_stalled_account_does_not_block_other_account_or_request_head() {
    let (_dir, app, user, a, slow) = ready().await;
    slow.delay_ms.store(30_000, Ordering::SeqCst);
    let mut input = app.store.credential(&a).await.unwrap();
    input.email = "second@example.invalid".into();
    let account = app.store.connect(&user, &input).await.unwrap();
    let second = app.store.authorize(&user, &account.id).await.unwrap();
    let fast = Arc::new(PagedProvider {
        base: Arc::new(InboxProvider {
            account: second.id().into(),
            count: 0,
            delay: 0,
            scans: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        }),
        limited: true,
        visited: Mutex::default(),
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        heads: AtomicBool::new(true),
        head_calls: AtomicUsize::new(0),
        arrival_uid: AtomicUsize::new(0),
        idle_hints: AtomicBool::new(false),
        delay_ms: AtomicUsize::new(10),
        cold_rows: AtomicUsize::new(0),
        mutations: AtomicUsize::new(0),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(second.id().into(), fast.clone());
    app.mail.probe(&app.store, &second).await.unwrap();
    let job = tokio::spawn(crate::executor::run(app.clone()));
    wait_pages(&slow, 1, 3).await;
    wait_pages(&fast, 3, 3).await; // fourth turn is the independent INBOX head
    assert_eq!(fast.head_calls.load(Ordering::SeqCst), 1);
    assert_eq!(slow.visited.lock().await.len(), 1);
    assert_eq!(fast.maximum.load(Ordering::SeqCst), 1);
    let lane = app.mail.sync_lane(&second).await;
    let guard = tokio::time::timeout(Duration::from_secs(1), lane.lock())
        .await
        .unwrap();
    drop(guard);
    job.abort();
    let _ = job.await;
    let before = fast.visited.lock().await.len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fast.visited.lock().await.len(), before);
    assert!(app
        .store
        .inbox_coverage(&a)
        .await
        .unwrap()
        .inbox
        .unwrap()
        .last_scanned_at
        .is_none());
}

#[tokio::test]
async fn executor_serial_read_intent_waits_for_current_page_not_entire_burst() {
    let (_dir, app, _, a, provider) = ready().await;
    provider.cold_rows.store(1, Ordering::SeqCst);
    crate::sync::tick(&app, &a).await.unwrap();
    let message = app.store.catalogue(&a).await.unwrap().remove(0);
    provider.delay_ms.store(8_000, Ordering::SeqCst);
    let job = tokio::spawn(crate::executor::run(app.clone()));
    wait_pages(&provider, 2, 3).await;
    let queued_at = Instant::now();
    app.store
        .operation(
            &a,
            Operation {
                id: "disposable-read".into(),
                account_id: a.id().into(),
                target_id: message.id.clone(),
                expected_revision: message.revision,
                change: Change::Read { value: false },
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(app.store.message(&a, &message.id).await.unwrap().unread);
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
    // Reconciliation owns the current provider page. The due mutation waits
    // for that page, not for four pages or for a new metadata admission tick.
    tokio::time::timeout(Duration::from_secs(11), async {
        while app
            .store
            .operation_status(&a, "disposable-read")
            .await
            .unwrap()
            .status
            != OperationStatus::Applied
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(queued_at.elapsed() < Duration::from_secs(11));
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
    assert!(provider.visited.lock().await.len() <= 3);
    job.abort();
    let _ = job.await;
}

#[tokio::test]
async fn executor_third_account_gets_freed_slot_before_tick_and_global_limit_is_two() {
    let (_dir, app, user, a, first) = ready().await;
    first.delay_ms.store(200, Ordering::SeqCst);
    let mut providers = vec![first];
    for i in 0..2 {
        let mut input = app.store.credential(&a).await.unwrap();
        input.email = format!("extra-{i}@example.invalid");
        let account = app.store.connect(&user, &input).await.unwrap();
        let scope = app.store.authorize(&user, &account.id).await.unwrap();
        let provider = Arc::new(PagedProvider {
            base: Arc::new(InboxProvider {
                account: scope.id().into(),
                count: 0,
                delay: 0,
                scans: AtomicUsize::new(0),
                fail: AtomicBool::new(false),
            }),
            limited: true,
            visited: Mutex::default(),
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            heads: AtomicBool::new(false),
            head_calls: AtomicUsize::new(0),
            arrival_uid: AtomicUsize::new(0),
            idle_hints: AtomicBool::new(false),
            delay_ms: AtomicUsize::new(200),
            cold_rows: AtomicUsize::new(0),
            mutations: AtomicUsize::new(0),
        });
        app.mail
            .providers
            .lock()
            .await
            .insert(scope.id().into(), provider.clone());
        app.mail.probe(&app.store, &scope).await.unwrap();
        providers.push(provider);
    }
    let job = tokio::spawn(crate::executor::run(app.clone()));
    let mut maximum = 0;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let active: usize = providers
                .iter()
                .map(|p| p.active.load(Ordering::SeqCst))
                .sum();
            maximum = maximum.max(active);
            assert!(active <= 2);
            let mut started = 0;
            for provider in &providers {
                started += usize::from(!provider.visited.lock().await.is_empty());
            }
            if started == 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(maximum, 2);
    assert!(providers
        .iter()
        .all(|p| p.maximum.load(Ordering::SeqCst) == 1));
    job.abort();
    let _ = job.await;
}

#[tokio::test]
#[ignore = "finite synthetic cold scheduler/storage benchmark, no real account"]
async fn cold_scheduler_storage_benchmark() {
    for replicate in 0..3 {
        let (_dir, app, _, a, provider) = ready().await;
        provider.cold_rows.store(200, Ordering::SeqCst);
        provider.delay_ms.store(150, Ordering::SeqCst);
        let started = Instant::now();
        let job = tokio::spawn(crate::executor::run(app.clone()));
        tokio::time::timeout(Duration::from_secs(100), async {
            loop {
                let mut complete = true;
                for folder in ["INBOX", "Archive", "Sent"] {
                    complete &= app.store.scan_state(&a, folder).await.unwrap().is_none();
                }
                if complete {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let elapsed_ms = started.elapsed().as_millis();
        job.abort();
        let _ = job.await;
        let db = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(
                    _dir.path()
                        .join("accounts")
                        .join(format!("{}.sqlite", a.id())),
                )
                .read_only(true),
        )
        .await
        .unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM messages")
            .fetch_one(&db)
            .await
            .unwrap();
        let memberships: i64 = sqlx::query_scalar("SELECT count(*) FROM folder_memberships")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(rows, 1800);
        assert_eq!(memberships, 1800);
        db.close().await;
        assert_eq!(provider.visited.lock().await.len(), 9);
        assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
        assert_eq!(
            app.store.inbox_coverage(&a).await.unwrap().state,
            CoverageState::Partial
        );
        println!(
            "COLD_SCHEDULER {}",
            serde_json::json!({"replicate":replicate,"elapsed_ms":elapsed_ms,"rows":1800,"pages":9,"rows_per_page":200,"header_padding_bytes":2048,"provider_delay_per_page_ms":150,"max_account_concurrency":1,"limited":true,"fixture":"in-process synthetic provider, real executor/SQLite/ingestion; not wire or live Gmail"})
        );
    }
}
