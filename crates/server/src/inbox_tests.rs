use super::*;
use axum::{
    extract::{Query, State},
    Extension,
};
use std::sync::atomic::AtomicBool;
use std::time::Instant;
#[path = "inbox_diagnostics_tests.rs"]
mod diagnostics_tests;
#[path = "reconcile_tests.rs"]
mod reconcile_tests;
#[path = "release_pipeline_tests.rs"]
mod release_pipeline_tests;
#[path = "starred_tests.rs"]
mod starred_tests;
#[path = "sync_fairness_tests.rs"]
mod sync_fairness_tests;

struct InboxProvider {
    account: String,
    count: usize,
    delay: u64,
    scans: AtomicUsize,
    fail: AtomicBool,
}
#[async_trait]
impl p::MailProvider for InboxProvider {
    fn account_id(&self) -> &str {
        &self.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        Ok(p::Probe {
            capabilities: vec![],
            folders: vec![p::Folder {
                name: "INBOX".into(),
                selectable: true,
                attributes: vec![],
            }],
            full_search_coverage: true,
        })
    }
    async fn scan(
        &self,
        folder: &str,
        _: &p::Search,
        token: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(self.delay)).await;
        if self.fail.load(Ordering::SeqCst) {
            return Err(p::error(p::ErrorKind::Retryable, "fixture failure"));
        }
        assert!(token.is_none());
        let rows = (0..self.count).map(|i| {
            let reference = p::MessageRef { graph_id: None, account_id: self.account.clone(), folder: folder.into(), uidvalidity: 1, uid: i as u32+1 };
            let headers = format!("From: Fixture <fixture@example.invalid>\r\nTo: Owner <owner@example.invalid>\r\nSubject: Synthetic {i}\r\nMessage-ID: <m{i}@example.invalid>\r\n\r\n").into_bytes();
            p::Metadata { identity: p::Identity::Imap(reference.clone()), reference, gmail_thread_id: None, labels: vec![], seen: true, starred: false, size: headers.len() as u32, internal_date: "29-Oct-2026 00:00:00 +0000".into(), headers }
        }).collect();
        Ok(p::Page {
            rows,
            next: None,
            uidvalidity: 1,
            full_coverage: true,
        })
    }
    async fn search(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        self.scan(f, q, t, n).await
    }
    async fn fetch(&self, _: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        panic!("inbox tests never read bodies")
    }
    async fn mutate(&self, _: &p::MessageRef, _: p::Mutation) -> p::Result<p::MutationResult> {
        panic!("no provider mutations")
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        unreachable!()
    }
    async fn submit(
        &self,
        _: &p::mime::PreparedMessage,
        _: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        panic!("no send")
    }
    async fn reconcile_sent(&self, _: &str, _: &[u8]) -> p::Result<p::SentOutcome> {
        panic!("no filing")
    }
}
async fn setup(
    count: usize,
    delay: u64,
) -> (
    tempfile::TempDir,
    crate::App,
    String,
    dakia_storage::AuthorizedAccount,
    Arc<InboxProvider>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [29; 32]).await.unwrap();
    let user = store.user("fixture", "inbox").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let scope = store.authorize(&user, &account.id).await.unwrap();
    let provider = Arc::new(InboxProvider {
        account: account.id.clone(),
        count,
        delay,
        scans: AtomicUsize::new(0),
        fail: AtomicBool::new(false),
    });
    let app = crate::App {
        store,
        config: Arc::new(crate::Config {
            origin: "https://dakia.example.invalid".into(),
            issuer: "https://id.example.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture-only".into(),
            web_root: dir.path().to_string_lossy().into(),
        }),
        mail: MailRuntime::default(),
    };
    app.mail
        .providers
        .lock()
        .await
        .insert(account.id, provider.clone());
    (dir, app, user, scope, provider)
}
async fn search(
    app: &crate::App,
    user: &str,
    request: serde_json::Value,
) -> crate::Result<SearchPage> {
    Ok(crate::search::search(
        State(app.clone()),
        Extension(crate::Identity {
            user: user.into(),
            csrf: "fixture".into(),
            token_hash: "fixture".into(),
        }),
        Query(serde_json::from_value(request).unwrap()),
    )
    .await?
    .0)
}
async fn wait_refresh(app: &crate::App, a: &dakia_storage::AuthorizedAccount) {
    let slot = app
        .mail
        .inbox_refreshes
        .lock()
        .await
        .get(a.id())
        .unwrap()
        .clone();
    let _done = slot.lock().await;
}
#[tokio::test]
async fn failed_refresh_retains_cached_rows_and_removal_during_io_cannot_recreate_account() {
    let (dir, app, user, a, provider) = setup(1, 100).await;
    let req = serde_json::json!({"q":"folder:INBOX", "accounts":a.id()});
    search(&app, &user, req.clone()).await.unwrap();
    wait_refresh(&app, &a).await;
    app.store.reset_scan(&a, "INBOX").await.unwrap();
    app.mail.inbox_refreshes.lock().await.clear();
    provider.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        search(&app, &user, req.clone()).await.unwrap().rows.len(),
        1
    );
    wait_refresh(&app, &a).await;
    let failed = search(&app, &user, req.clone()).await.unwrap();
    assert_eq!(failed.rows.len(), 1);
    assert_eq!(failed.coverage[0].state, CoverageState::Error);
    app.mail.inbox_refreshes.lock().await.clear();
    provider.fail.store(false, Ordering::SeqCst);
    search(&app, &user, req.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    app.store
        .remove_account(&a, "owner@example.invalid")
        .await
        .unwrap();
    wait_refresh(&app, &a).await;
    assert!(search(&app, &user, req).await.is_err());
    assert!(!dir
        .path()
        .join("accounts")
        .join(format!("{}.sqlite", a.id()))
        .exists());
}
#[tokio::test]
async fn cache_first_is_fast_during_provider_io_and_respects_intents_and_isolation() {
    let (_dir, app, user, a, provider) = setup(50, 300).await;
    let req = serde_json::json!({"q":"folder:INBOX", "accounts": a.id()});
    let start = Instant::now();
    let cold = search(&app, &user, req.clone()).await.unwrap();
    assert!(cold.rows.is_empty());
    assert!(cold.pending);
    assert!(start.elapsed() < Duration::from_millis(250));
    for _ in 0..10 {
        search(&app, &user, req.clone()).await.unwrap();
    }
    wait_refresh(&app, &a).await;
    assert_eq!(provider.scans.load(Ordering::SeqCst), 1);
    let lane = app.mail.reconciliation_lane(&a).await;
    let _blocked = lane.lock().await;
    let start = Instant::now();
    let warm = search(&app, &user, req.clone()).await.unwrap();
    println!(
        "INBOX_OPTIMIZED warm_50_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    assert_eq!(warm.rows.len(), 50);
    assert!(!warm.pending);
    assert!(start.elapsed() < Duration::from_millis(250));
    let m = &warm.rows[0];
    let read = Operation {
        id: "read".into(),
        account_id: a.id().into(),
        target_id: m.id.clone(),
        expected_revision: m.revision,
        change: Change::Read { value: false },
        status: OperationStatus::Pending,
        revision: 0,
    };
    app.store.operation(&a, read).await.unwrap();
    let m = app.store.message(&a, &m.id).await.unwrap();
    app.store
        .operation(
            &a,
            Operation {
                id: "archive".into(),
                account_id: a.id().into(),
                target_id: m.id.clone(),
                expected_revision: m.revision,
                change: Change::Archive,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    let next = search(&app, &user, req.clone()).await.unwrap();
    assert_eq!(next.rows.len(), 49);
    assert!(!next.rows.iter().any(|r| r.id == m.id));
    let other = app.store.user("fixture", "other").await.unwrap();
    assert_eq!(
        search(&app, &other, req.clone()).await.unwrap_err().0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(provider.scans.load(Ordering::SeqCst), 1);
    app.store
        .inbox_refresh_result(&a, Some("provider_unavailable"))
        .await
        .unwrap();
    let failed = search(&app, &user, req).await.unwrap();
    assert_eq!(failed.rows.len(), 49);
    assert_eq!(failed.coverage[0].state, CoverageState::Error);
}
#[tokio::test]
async fn provider_ingestion_before_after_comparable_synthetic_probe() {
    let (_dir, app, user, a, _provider) = setup(50, 0).await;
    let input = app.store.credential(&a).await.unwrap();
    let second = app.store.connect(&user, &input).await.unwrap();
    app.mail.providers.lock().await.insert(
        second.id.clone(),
        Arc::new(InboxProvider {
            account: second.id.clone(),
            count: 50,
            delay: 0,
            scans: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        }),
    );
    // Extra seen predicate forces the unchanged provider-backed search path,
    // allowing comparison against the original zero-network ingestion benchmark.
    let req = serde_json::json!({"q":"folder:INBOX is:read", "accounts":format!("{},{}",a.id(),second.id)});
    for repeat in 0..3 {
        let start = Instant::now();
        let result = search(&app, &user, req.clone()).await.unwrap();
        assert_eq!(result.rows.len(), 100);
        println!(
            "INBOX_INGESTION repeat={repeat} rows=100 elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.
        );
    }
    let start = Instant::now();
    let cached = search(
        &app,
        &user,
        serde_json::json!({"q":"folder:INBOX","accounts":format!("{},{}",a.id(),second.id)}),
    )
    .await
    .unwrap();
    assert_eq!(cached.rows.len(), 100);
    println!(
        "INBOX_CACHED rows=100 elapsed_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    wait_refresh(&app, &a).await;
    let b = app.store.authorize(&user, &second.id).await.unwrap();
    wait_refresh(&app, &b).await;
}
