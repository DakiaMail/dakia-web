//! Synthetic multi-page Gmail/IMAP through the real scheduler and refresh lane.
use super::*;
#[path = "head_sync_tests.rs"]
mod head_sync_tests;
#[path = "idle_pipeline_tests.rs"]
mod idle_pipeline_tests;
#[path = "scheduler_tests.rs"]
mod scheduler_tests;

struct PagedProvider {
    base: Arc<InboxProvider>,
    limited: bool,
    visited: Mutex<Vec<(String, u32)>>,
    active: AtomicUsize,
    maximum: AtomicUsize,
    heads: AtomicBool,
    head_calls: AtomicUsize,
    arrival_uid: AtomicUsize,
    idle_hints: AtomicBool,
    delay_ms: AtomicUsize,
    cold_rows: AtomicUsize,
    mutations: AtomicUsize,
}
#[async_trait]
impl p::MailProvider for PagedProvider {
    fn supports_head(&self) -> bool {
        self.heads.load(Ordering::SeqCst)
    }
    async fn head(
        &self,
        folder: &str,
        after: Option<&p::HeadCheckpoint>,
    ) -> p::Result<p::HeadPage> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        self.head_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(10)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.base.fail.load(Ordering::SeqCst) {
            return Err(p::error(p::ErrorKind::Retryable, "head fixture failure"));
        }
        let uid = 101 + self.arrival_uid.load(Ordering::SeqCst) as u32;
        let reference = p::MessageRef {
            account_id: self.base.account.clone(),
            folder: folder.into(),
            uidvalidity: 1,
            uid,
        };
        let rows = if after.is_none_or(|c| c.uid < uid) {
            vec![p::Metadata {
            identity: p::Identity::Imap(reference.clone()), reference, gmail_thread_id: None, labels: vec![],
            seen: true, starred: false, size: 100, internal_date: if uid == 101 { "29-Oct-2026 00:00:00 +0000".into() } else { (chrono::Utc::now() + chrono::Duration::hours(1)).format("%d-%b-%Y %H:%M:%S %z").to_string() },
            headers: format!("From: fixture@example.invalid\r\nSubject: New head\r\nMessage-ID: <head-{uid}@example.invalid>\r\n\r\n").into_bytes(),
        }]
        } else {
            vec![]
        };
        Ok(p::HeadPage {
            rows,
            checkpoint: p::HeadCheckpoint {
                account_id: self.base.account.clone(),
                folder: folder.into(),
                uidvalidity: 1,
                uid,
            },
        })
    }
    fn account_id(&self) -> &str {
        &self.base.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        Ok(p::Probe {
            capabilities: vec![],
            folders: self.folders().await?,
            full_search_coverage: !self.limited,
        })
    }
    async fn folders(&self) -> p::Result<Vec<p::Folder>> {
        Ok(["INBOX", "Archive", "Sent"]
            .into_iter()
            .map(|name| p::Folder {
                name: name.into(),
                selectable: true,
                attributes: vec![],
            })
            .collect())
    }
    async fn scan(
        &self,
        folder: &str,
        query: &p::Search,
        token: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        if query.starred == Some(true) {
            return Ok(p::Page {
                rows: vec![],
                next: None,
                uidvalidity: 1,
                full_coverage: false,
            });
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        let cold_rows = self.cold_rows.load(Ordering::SeqCst);
        let page = token.map_or(0, |t| {
            if cold_rows == 0 {
                t.before_uid
            } else {
                (3 * cold_rows as u32 - t.before_uid) / cold_rows as u32 + 1
            }
        });
        self.visited.lock().await.push((folder.into(), page));
        tokio::time::sleep(Duration::from_millis(
            self.delay_ms.load(Ordering::SeqCst) as u64
        ))
        .await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if folder == "INBOX" && self.base.fail.load(Ordering::SeqCst) {
            return Err(p::error(p::ErrorKind::Retryable, "synthetic failure"));
        }
        let rows = (0..cold_rows).map(|i| {
            let uid = (3 - page) * cold_rows as u32 - i as u32;
            let reference = p::MessageRef { account_id: self.base.account.clone(), folder: folder.into(), uidvalidity: 1, uid };
            p::Metadata {
                identity: p::Identity::Imap(reference.clone()), reference,
                gmail_thread_id: None, labels: vec![], seen: true, starred: false, size: 5000,
                internal_date: "29-Oct-2026 00:00:00 +0000".into(),
                headers: format!("From: fixture@example.invalid\r\nSubject: Cold {folder} {uid}\r\nMessage-ID: <{folder}-{uid}@example.invalid>\r\nX-Padding: {}\r\n\r\n", "x".repeat(2048)).into_bytes(),
            }
        }).collect();
        Ok(p::Page {
            rows,
            uidvalidity: 1,
            full_coverage: !self.limited,
            next: (page < 2).then(|| p::PageToken {
                account_id: self.base.account.clone(),
                folder: folder.into(),
                uidvalidity: 1,
                upper_uid: if cold_rows == 0 {
                    100
                } else {
                    3 * cold_rows as u32
                },
                before_uid: if cold_rows == 0 {
                    page + 1
                } else {
                    (2 - page) * cold_rows as u32 + 1
                },
                query: p::Search::default(),
                modseq: None,
            }),
        })
    }
    async fn search(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        if q.starred == Some(true) {
            return Ok(p::Page {
                rows: vec![],
                next: None,
                uidvalidity: 1,
                full_coverage: false,
            });
        }
        self.scan(f, q, t, n).await
    }
    async fn fetch(&self, _: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        panic!("no body reads")
    }
    async fn mutate(
        &self,
        _: &p::MessageRef,
        mutation: p::Mutation,
    ) -> p::Result<p::MutationResult> {
        assert!(
            matches!(mutation, p::Mutation::Seen(_)),
            "only disposable read-intent test"
        );
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(p::MutationResult {
            destination: None,
            destination_reference: None,
            requires_rescan: false,
        })
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        Ok(self.idle_hints.load(Ordering::SeqCst))
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
async fn paged(
    limited: bool,
) -> (
    tempfile::TempDir,
    crate::App,
    String,
    dakia_storage::AuthorizedAccount,
    Arc<PagedProvider>,
) {
    let (dir, app, user, original, base) = setup(0, 0).await;
    let mut input = app.store.credential(&original).await.unwrap();
    input.provider = if limited {
        ProviderKind::Gmail
    } else {
        ProviderKind::Imap
    };
    let account = app.store.connect(&user, &input).await.unwrap();
    let a = app.store.authorize(&user, &account.id).await.unwrap();
    let provider = Arc::new(PagedProvider {
        base: Arc::new(InboxProvider {
            account: a.id().into(),
            count: 0,
            delay: 0,
            scans: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        }),
        limited,
        visited: Mutex::default(),
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        heads: AtomicBool::new(false),
        head_calls: AtomicUsize::new(0),
        arrival_uid: AtomicUsize::new(0),
        idle_hints: AtomicBool::new(false),
        delay_ms: AtomicUsize::new(10),
        cold_rows: AtomicUsize::new(0),
        mutations: AtomicUsize::new(0),
    });
    assert_eq!(base.scans.load(Ordering::SeqCst), 0);
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    app.mail.probe(&app.store, &a).await.unwrap();
    (dir, app, user, a, provider)
}
#[tokio::test]
async fn gmail_limited_and_imap_complete_scans_cool_down_while_all_folders_advance() {
    for limited in [true, false] {
        let (_dir, app, _, a, provider) = paged(limited).await;
        for _ in 0..20 {
            let before = provider.visited.lock().await.len();
            crate::sync::tick(&app, &a).await.unwrap();
            assert!(provider.visited.lock().await.len() <= before + 1);
        }
        let visits = provider.visited.lock().await.clone();
        assert_eq!(
            visits.len(),
            9,
            "exactly three pages per folder, no cycling"
        );
        assert_eq!(
            &visits[..5],
            &[
                ("INBOX".into(), 0),
                ("Archive".into(), 0),
                ("INBOX".into(), 1),
                ("Sent".into(), 0),
                ("INBOX".into(), 2)
            ]
        );
        for folder in ["INBOX", "Archive", "Sent"] {
            assert_eq!(
                visits
                    .iter()
                    .filter(|(f, _)| f == folder)
                    .map(|(_, p)| *p)
                    .collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
            assert!(app.store.scan_state(&a, folder).await.unwrap().is_none());
        }
        let coverage = app.store.inbox_coverage(&a).await.unwrap();
        assert_eq!(
            coverage.state,
            if limited {
                CoverageState::Partial
            } else {
                CoverageState::Complete
            }
        );
        let freshness = coverage.inbox.unwrap();
        assert!(freshness.last_scanned_at.is_some());
        assert!(!freshness.refreshing);
        let reopened = Store::open(_dir.path(), [29; 32]).await.unwrap();
        let reopened_a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
        assert!(reopened
            .scan_state(&reopened_a, "INBOX")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            reopened
                .inbox_coverage(&reopened_a)
                .await
                .unwrap()
                .inbox
                .unwrap(),
            freshness
        );
    }
}
#[tokio::test]
async fn failing_inbox_does_not_starve_backfill_and_recovery_is_honest() {
    let (_dir, app, _, a, provider) = paged(true).await;
    provider.base.fail.store(true, Ordering::SeqCst);
    for _ in 0..12 {
        crate::sync::tick(&app, &a).await.unwrap();
    }
    let visits = provider.visited.lock().await.clone();
    assert_eq!(visits.iter().filter(|(f, _)| f == "Archive").count(), 3);
    assert_eq!(visits.iter().filter(|(f, _)| f == "Sent").count(), 3);
    assert_eq!(
        app.store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Error
    );
    assert_eq!(
        app.store
            .inbox_coverage(&a)
            .await
            .unwrap()
            .inbox
            .unwrap()
            .last_scanned_at,
        None
    );
    provider.base.fail.store(false, Ordering::SeqCst);
    for _ in 0..3 {
        crate::sync::tick(&app, &a).await.unwrap();
    }
    assert_eq!(
        app.store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Partial
    );
    assert!(
        !app.store
            .inbox_coverage(&a)
            .await
            .unwrap()
            .inbox
            .unwrap()
            .refreshing
    );
}
#[tokio::test]
async fn concurrent_background_and_request_refreshes_serialize_and_preserve_cooldown() {
    let (_dir, app, user, a, provider) = paged(true).await;
    let req = serde_json::json!({"q":"folder:INBOX","accounts":a.id()});
    let background = async {
        for _ in 0..12 {
            crate::sync::tick(&app, &a).await.unwrap();
        }
    };
    let requests = async {
        for _ in 0..12 {
            search(&app, &user, req.clone()).await.unwrap();
        }
    };
    tokio::join!(background, requests);
    wait_refresh(&app, &a).await;
    assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider
            .visited
            .lock()
            .await
            .iter()
            .filter(|(f, _)| f == "INBOX")
            .count(),
        3
    );
    let result = search(&app, &user, req).await.unwrap();
    assert!(!result.pending);
    assert_eq!(result.coverage[0].state, CoverageState::Partial);
    // A different account's initial scan/freshness must remain untouched.
    let accounts = app.store.accounts(&user).await.unwrap();
    let other = accounts.iter().find(|b| b.id != a.id()).unwrap();
    let other = app.store.authorize(&user, &other.id).await.unwrap();
    assert_eq!(
        app.store.inbox_coverage(&other).await.unwrap().state,
        CoverageState::Pending
    );
}

#[tokio::test]
async fn expired_completed_folders_cannot_restart_ahead_of_unfinished_history() {
    for limited in [true, false] {
        let (dir, app, _, a, provider) = paged(limited).await;
        provider.cold_rows.store(200, Ordering::SeqCst);
        let db = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(
                dir.path()
                    .join("accounts")
                    .join(format!("{}.sqlite", a.id())),
            ),
        )
        .await
        .unwrap();
        for _ in 0..20 {
            // Simulate folders taking longer than the real 60-second cooldown.
            // Only disposable scheduler time is advanced, not a live mailbox.
            sqlx::query("UPDATE folder_scans SET last_scanned_at=0 WHERE continuation IS NULL AND (completed_at IS NOT NULL OR coverage_limited=1)")
                .execute(&db).await.unwrap();
            crate::sync::tick(&app, &a).await.unwrap();
            if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM messages")
                .fetch_one(&db)
                .await
                .unwrap()
                == 1800
            {
                break;
            }
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM messages")
                .fetch_one(&db)
                .await
                .unwrap(),
            1800
        );
        let visits = provider.visited.lock().await.clone();
        assert_eq!(
            visits.len(),
            9,
            "completed folders must yield to unfinished traversal: {visits:?}"
        );
        for folder in ["INBOX", "Archive", "Sent"] {
            assert_eq!(
                visits
                    .iter()
                    .filter(|(f, _)| f == folder)
                    .map(|(_, p)| *p)
                    .collect::<Vec<_>>(),
                vec![0, 1, 2]
            );
        }
        // Normal periodic reconciliation resumes after all traversals finish.
        crate::sync::tick(&app, &a).await.unwrap();
        assert_eq!(provider.visited.lock().await.len(), 10);
        assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
        let reopened = Store::open(dir.path(), [29; 32]).await.unwrap();
        let reopened_a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
        assert_eq!(
            reopened
                .terminal_scan_folders(&reopened_a)
                .await
                .unwrap()
                .len(),
            2
        );
        let other = app
            .store
            .accounts(a.user_id())
            .await
            .unwrap()
            .into_iter()
            .find(|b| b.id != a.id())
            .unwrap();
        let other = app.store.authorize(a.user_id(), &other.id).await.unwrap();
        assert!(app
            .store
            .terminal_scan_folders(&other)
            .await
            .unwrap()
            .is_empty());
    }
}

#[tokio::test]
async fn unfinished_failure_cannot_suppress_routine_reconciliation_forever() {
    let (dir, app, _, a, provider) = paged(true).await;
    for _ in 0..20 {
        crate::sync::tick(&app, &a).await.unwrap();
    }
    assert_eq!(provider.visited.lock().await.len(), 9);
    app.store.reset_scan(&a, "INBOX").await.unwrap();
    provider.base.fail.store(true, Ordering::SeqCst);
    let db = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(
            dir.path()
                .join("accounts")
                .join(format!("{}.sqlite", a.id())),
        ),
    )
    .await
    .unwrap();
    // Keep routine folders due while the unfinished INBOX always fails.
    for _ in 0..70 {
        sqlx::query("UPDATE folder_scans SET last_scanned_at=0 WHERE folder<>'INBOX'")
            .execute(&db)
            .await
            .unwrap();
        crate::sync::tick(&app, &a).await.unwrap();
    }
    let visits = provider.visited.lock().await;
    for folder in ["Archive", "Sent"] {
        assert!(
            visits.iter().filter(|(f, _)| f == folder).count() > 3,
            "routine {folder} was starved"
        );
    }
    assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
}
