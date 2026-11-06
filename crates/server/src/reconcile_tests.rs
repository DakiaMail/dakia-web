use super::*;
#[tokio::test]
async fn executor_runs_metadata_for_two_accounts_without_serial_head_of_line_and_cancels_on_shutdown(
) {
    let (_dir, app, user, a, first) = setup(0, 900).await;
    app.mail.probe(&app.store, &a).await.unwrap();
    let mut input = app.store.credential(&a).await.unwrap();
    input.email = "second@example.invalid".into();
    let account = app.store.connect(&user, &input).await.unwrap();
    let b = app.store.authorize(&user, &account.id).await.unwrap();
    let second = Arc::new(InboxProvider {
        account: b.id().into(),
        count: 0,
        delay: 900,
        scans: AtomicUsize::new(0),
        fail: AtomicBool::new(false),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(b.id().into(), second.clone());
    app.mail.probe(&app.store, &b).await.unwrap();
    let task = tokio::spawn(crate::executor::run(app.clone()));
    tokio::time::timeout(Duration::from_millis(700), async {
        while first.scans.load(Ordering::SeqCst) == 0 || second.scans.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both accounts start metadata before either 900ms scan completes");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let counts = (
        first.scans.load(Ordering::SeqCst),
        second.scans.load(Ordering::SeqCst),
    );
    tokio::time::sleep(Duration::from_millis(950)).await;
    assert_eq!(
        counts,
        (
            first.scans.load(Ordering::SeqCst),
            second.scans.load(Ordering::SeqCst)
        ),
        "shutdown cannot detach metadata workers"
    );
}

struct DeltaProvider {
    base: Arc<InboxProvider>,
    mode: AtomicUsize,
    checks: AtomicUsize,
    pages: AtomicUsize,
}
#[async_trait]
impl p::MailProvider for DeltaProvider {
    fn account_id(&self) -> &str {
        &self.base.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        self.base.probe().await
    }
    async fn scan(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        self.base.scan(f, q, t, n).await
    }
    async fn snapshot_unchanged(&self, checkpoint: &p::ModseqCheckpoint) -> p::Result<bool> {
        assert_eq!(checkpoint.account_id, self.base.account);
        self.checks.fetch_add(1, Ordering::SeqCst);
        if self.mode.load(Ordering::SeqCst) == 11 {
            return Err(p::error(
                p::ErrorKind::Retryable,
                "IMAP operation deadline exceeded",
            ));
        }
        Ok(self.mode.load(Ordering::SeqCst) == 10)
    }
    async fn reconcile(
        &self,
        folder: &str,
        _: Option<&p::PageToken>,
        _: usize,
        known: &dyn p::MetadataLookup,
        _: Option<&p::ModseqCheckpoint>,
    ) -> p::Result<p::ReconcilePage> {
        self.pages.fetch_add(1, Ordering::SeqCst);
        let reference = |uid| p::MessageRef {
            account_id: self.base.account.clone(),
            folder: folder.into(),
            uidvalidity: 1,
            uid,
        };
        let r = reference(1);
        assert!(known.identity_known(&p::Identity::Imap(r.clone())).await?);
        assert!(known.reference_known(&r).await?);
        let mut rows = vec![p::MetadataChange::Mutable(p::MutableMetadata {
            identity: p::Identity::Imap(r.clone()),
            reference: r,
            labels: vec![],
            seen: false,
            starred: false,
        })];
        if self.mode.load(Ordering::SeqCst) == 0 {
            rows.push(p::MetadataChange::Unchanged(reference(2)));
        }
        Ok(p::ReconcilePage {
            rows,
            next: None,
            uidvalidity: 1,
            full_coverage: true,
            checkpoint: Some(p::ModseqCheckpoint {
                account_id: self.base.account.clone(),
                folder: folder.into(),
                uidvalidity: 1,
                modseq: 90,
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
        self.scan(f, q, t, n).await
    }
    async fn fetch(&self, _: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        panic!("no body reads")
    }
    async fn mutate(&self, _: &p::MessageRef, _: p::Mutation) -> p::Result<p::MutationResult> {
        panic!("no mail mutations")
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        panic!("no IDLE")
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

#[tokio::test]
async fn mutable_checkpoint_transaction_rolls_back_on_fault_then_replays_preserving_headers_and_intents(
) {
    let (dir, app, user, a, base) = setup(2, 0).await;
    let initial = app
        .mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    let first = initial
        .rows
        .iter()
        .find(|m| m.subject == "Synthetic 0")
        .unwrap()
        .clone();
    app.store
        .operation(
            &a,
            Operation {
                id: "pending-star".into(),
                account_id: a.id().into(),
                target_id: first.id.clone(),
                expected_revision: first.revision,
                change: Change::Star { value: true },
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    let before = app.store.message(&a, &first.id).await.unwrap();
    let provider = Arc::new(DeltaProvider {
        base,
        mode: AtomicUsize::new(0),
        checks: AtomicUsize::new(0),
        pages: AtomicUsize::new(0),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    sqlx::query("CREATE TRIGGER fail_checkpoint BEFORE INSERT ON provider_documents WHEN NEW.key='sync:modseq:INBOX' BEGIN SELECT RAISE(ABORT,'synthetic crash'); END").execute(&pool).await.unwrap();
    assert!(app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .is_err());
    assert!(app
        .store
        .provider_document(&a, "sync:modseq:INBOX")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        app.store.message(&a, &first.id).await.unwrap(),
        before,
        "mutable row and token must roll back together"
    );
    sqlx::query("DROP TRIGGER fail_checkpoint")
        .execute(&pool)
        .await
        .unwrap();
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    assert_eq!(
        page.rows.len(),
        2,
        "delta omission does not omit unchanged snapshot members"
    );
    let updated = app.store.message(&a, &first.id).await.unwrap();
    assert_eq!(updated.subject, before.subject);
    assert_eq!(updated.sender, before.sender);
    assert_eq!(updated.received_at, before.received_at);
    assert!(updated.unread);
    assert!(
        updated.starred,
        "pending local intent wins over provider flags"
    );
    let checkpoint = app
        .store
        .provider_document(&a, "sync:modseq:INBOX")
        .await
        .unwrap()
        .unwrap();
    let reopened = Store::open(dir.path(), [29; 32]).await.unwrap();
    let reopened_a = reopened.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        reopened
            .provider_document(&reopened_a, "sync:modseq:INBOX")
            .await
            .unwrap()
            .unwrap(),
        checkpoint
    );
    assert_eq!(
        reopened.message(&reopened_a, &first.id).await.unwrap(),
        updated
    );
    // Snapshot pruning is still separate and requires a complete traversal.
    let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    app.store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 2);
    app.store.reset_scan(&a, "INBOX").await.unwrap();
    let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    provider.mode.store(1, Ordering::SeqCst);
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    assert_eq!(
        app.store.inbox_rows(&a, None, None).await.unwrap().len(),
        2,
        "no pruning during ingestion"
    );
    app.store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert_eq!(
        app.store.inbox_rows(&a, None, None).await.unwrap().len(),
        1,
        "complete membership traversal detects expunge"
    );
    pool.close().await;
}

#[tokio::test]
async fn four_hours_of_unchanged_completed_snapshots_keep_generation_and_rows_without_full_rescans()
{
    let (dir, app, _, a, base) = setup(2, 0).await;
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    let provider = Arc::new(DeltaProvider {
        base,
        mode: AtomicUsize::new(0),
        checks: AtomicUsize::new(0),
        pages: AtomicUsize::new(0),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    app.mail
        .record_snapshot_page(&app.store, &a, "INBOX", generation, None)
        .await
        .unwrap();
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&db)
        .await
        .unwrap();
    provider.mode.store(10, Ordering::SeqCst);
    // Forty-eight five-minute eligibility windows, without wall-clock sleeps.
    for _ in 0..48 {
        sqlx::query(
            "UPDATE folder_scans SET last_scanned_at=unixepoch('now')-301 WHERE folder='INBOX'",
        )
        .execute(&db)
        .await
        .unwrap();
        assert_eq!(
            app.mail
                .prepare_scan(&app.store, &a, "INBOX")
                .await
                .unwrap(),
            (None, true)
        );
        assert_eq!(
            app.mail
                .prepare_scan(&app.store, &a, "INBOX")
                .await
                .unwrap(),
            (None, false)
        );
        assert_eq!(
            app.store.scan_generation(&a, "INBOX").await.unwrap(),
            Some(generation)
        );
        assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 2);
    }
    assert_eq!(provider.checks.load(Ordering::SeqCst), 48);
    assert_eq!(provider.pages.load(Ordering::SeqCst), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
            .fetch_one(&db)
            .await
            .unwrap(),
        events
    );
    // A failed check does not touch freshness, advance a generation or prune.
    sqlx::query("UPDATE folder_scans SET last_scanned_at=unixepoch('now')-301")
        .execute(&db)
        .await
        .unwrap();
    provider.mode.store(11, Ordering::SeqCst);
    assert!(app
        .mail
        .prepare_scan(&app.store, &a, "INBOX")
        .await
        .is_err());
    assert_eq!(
        app.store.completed_scan_due(&a, "INBOX").await.unwrap(),
        Some(generation)
    );
    // A real change falls back immediately to the existing full traversal.
    provider.mode.store(1, Ordering::SeqCst);
    let (scan, checked) = app
        .mail
        .prepare_scan(&app.store, &a, "INBOX")
        .await
        .unwrap();
    assert!(!checked);
    let (next, token) = scan.unwrap();
    assert_eq!(next, generation + 1);
    assert!(token.is_none());
    app.mail
        .record_snapshot_page(&app.store, &a, "INBOX", next, None)
        .await
        .unwrap();
    assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
    // Limited coverage and explicit reset never qualify for the shortcut.
    provider.mode.store(10, Ordering::SeqCst);
    let checks = provider.checks.load(Ordering::SeqCst);
    sqlx::query("UPDATE folder_scans SET completed_at=NULL,coverage_limited=1,last_scanned_at=unixepoch('now')-301").execute(&db).await.unwrap();
    assert!(app
        .mail
        .prepare_scan(&app.store, &a, "INBOX")
        .await
        .unwrap()
        .0
        .is_some());
    assert_eq!(provider.checks.load(Ordering::SeqCst), checks);
    app.store.reset_scan(&a, "INBOX").await.unwrap();
    assert!(app
        .mail
        .prepare_scan(&app.store, &a, "INBOX")
        .await
        .unwrap()
        .0
        .is_some());
    assert!(matches!(
        app.store
            .confirm_unchanged_scan(&a, "INBOX", generation)
            .await,
        Err(Error::Conflict)
    ));
}

#[tokio::test]
async fn completed_folder_checks_charge_one_scheduler_turn_and_failures_do_not_starve_other_folders(
) {
    let (dir, app, _, a, base) = setup(0, 0).await;
    let provider = Arc::new(DeltaProvider {
        base,
        mode: AtomicUsize::new(10),
        checks: AtomicUsize::new(0),
        pages: AtomicUsize::new(0),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    let folders: Vec<_> = (0..22)
        .map(|i| p::Folder {
            name: if i == 0 {
                "INBOX".into()
            } else {
                format!("Folder-{i}")
            },
            attributes: vec![],
            selectable: true,
        })
        .collect();
    app.store.put_provider_document(&a,"sync:folders",&serde_json::json!({"folders":folders,"index":0,"refreshed":chrono::Utc::now().timestamp(),"backfill_turn":false,"head_turn":false}).to_string()).await.unwrap();
    for folder in &folders {
        let (generation, _) = app
            .store
            .scan_state(&a, &folder.name)
            .await
            .unwrap()
            .unwrap();
        let page = SearchPage {
            rows: vec![],
            continuation: None,
            coverage: vec![AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec![folder.name.clone()],
                inbox: None,
                state: CoverageState::Complete,
                error: None,
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
        app.store
            .record_scan(&a, &folder.name, generation, &page)
            .await
            .unwrap();
        let checkpoint = p::ModseqCheckpoint {
            account_id: a.id().into(),
            folder: folder.name.clone(),
            uidvalidity: 1,
            modseq: 90,
        };
        app.store
            .put_provider_document(
                &a,
                &format!("sync:modseq:{}", folder.name),
                &serde_json::to_string(&checkpoint).unwrap(),
            )
            .await
            .unwrap();
    }
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE folder_scans SET last_scanned_at=unixepoch('now')-301")
        .execute(&db)
        .await
        .unwrap();
    for expected in 1..=22 {
        assert!(crate::sync::tick(&app, &a).await.unwrap());
        assert_eq!(provider.checks.load(Ordering::SeqCst), expected);
    }
    assert!(!crate::sync::tick(&app, &a).await.unwrap());
    assert_eq!(provider.checks.load(Ordering::SeqCst), 22);
    assert_eq!(provider.pages.load(Ordering::SeqCst), 0);
    provider.mode.store(11, Ordering::SeqCst);
    sqlx::query("UPDATE folder_scans SET last_scanned_at=unixepoch('now')-301")
        .execute(&db)
        .await
        .unwrap();
    for _ in 0..2 {
        assert!(!crate::sync::tick(&app, &a).await.unwrap());
    }
    let state: serde_json::Value = serde_json::from_str(
        &app.store
            .provider_document(&a, "sync:folders")
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["index"], 2,
        "failed Inbox check yielded the next turn to Folder-1"
    );
    assert_eq!(provider.checks.load(Ordering::SeqCst), 24);
    for folder in &folders {
        assert_eq!(
            app.store.scan_generation(&a, &folder.name).await.unwrap(),
            Some(1)
        );
    }
}
