//! Production orchestration and SQLite with a controlled provider boundary.
//! All addresses, bodies, accounts and failures are disposable fixtures.
use super::*;
use dakia_mail_provider::MailProvider;
#[path = "full_text_tests.rs"]
mod full_text_tests;
#[path = "rules_pipeline_tests.rs"]
mod rules_pipeline_tests;
#[path = "search_preview_tests.rs"]
mod search_preview_tests;
#[path = "sender_cleanup_pipeline_tests.rs"]
mod sender_cleanup_pipeline_tests;
#[path = "unsubscribe_pipeline_tests.rs"]
mod unsubscribe_pipeline_tests;

struct Controlled {
    base: Arc<InboxProvider>,
    page: Mutex<p::Page>,
    failure: Mutex<Option<p::ErrorKind>>,
    head_calls: AtomicUsize,
    fetch_calls: AtomicUsize,
    recovery_calls: AtomicUsize,
    recovered: Mutex<Option<p::Metadata>>,
    raw: Mutex<Vec<u8>>,
    mutations: Mutex<Vec<p::MessageRef>>,
    stale_inbox: AtomicBool,
    search_delay: Mutex<Duration>,
}
#[async_trait]
impl p::MailProvider for Controlled {
    fn account_id(&self) -> &str {
        &self.base.account
    }
    fn supports_head(&self) -> bool {
        true
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        self.base.probe().await
    }
    async fn scan(
        &self,
        folder: &str,
        _: &p::Search,
        _: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        self.base.scans.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(*self.search_delay.lock().await).await;
        if let Some(kind) = *self.failure.lock().await {
            return Err(p::error(kind, "fixture-only"));
        }
        let mut page = self.page.lock().await.clone();
        if std::env::var_os("DAKIA_CLEANUP_FIXTURE").is_some() {
            page.rows.retain(|m| m.reference.folder == folder);
        }
        Ok(page)
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
    async fn head(
        &self,
        folder: &str,
        after: Option<&p::HeadCheckpoint>,
    ) -> p::Result<p::HeadPage> {
        self.head_calls.fetch_add(1, Ordering::SeqCst);
        let page = self.scan(folder, &p::Search::default(), None, 50).await?;
        let uid = page
            .rows
            .iter()
            .map(|m| m.reference.uid)
            .max()
            .unwrap_or_else(|| after.map_or(0, |c| c.uid));
        Ok(p::HeadPage {
            rows: page.rows,
            checkpoint: p::HeadCheckpoint {
                account_id: self.base.account.clone(),
                folder: folder.into(),
                uidvalidity: page.uidvalidity,
                uid,
            },
        })
    }
    async fn fetch(&self, _: &p::MessageRef, limit: u32) -> p::Result<Vec<u8>> {
        self.fetch_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(kind) = *self.failure.lock().await {
            return Err(p::error(kind, "fixture-only"));
        }
        let raw = self.raw.lock().await;
        if raw.len() > limit as usize {
            return Err(p::error(
                p::ErrorKind::InvalidInput,
                "message exceeds content limit",
            ));
        }
        Ok(raw.clone())
    }
    async fn resolve_gmail_message(
        &self,
        id: u64,
        folders: &[String],
    ) -> p::Result<Option<p::Metadata>> {
        assert!(folders.len() <= 4);
        assert!(id > 0);
        self.recovery_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.recovered.lock().await.clone())
    }
    async fn mutate(&self, r: &p::MessageRef, change: p::Mutation) -> p::Result<p::MutationResult> {
        self.mutations.lock().await.push(r.clone());
        if let Some(kind) = *self.failure.lock().await {
            return Err(p::error(kind, "fixture-only"));
        }
        if self.stale_inbox.load(Ordering::SeqCst) && r.folder == "INBOX" {
            return Err(p::error(
                p::ErrorKind::StaleReference,
                "fixture expunge before write",
            ));
        }
        let mut page = self.page.lock().await;
        let gmail = page
            .rows
            .iter()
            .any(|m| m.reference == *r && matches!(m.identity, p::Identity::Gmail { .. }));
        for m in page.rows.iter_mut().filter(|m| m.reference == *r) {
            match &change {
                p::Mutation::Seen(v) => m.seen = *v,
                p::Mutation::Starred(v) => m.starred = *v,
                _ => {}
            }
        }
        if matches!(
            &change,
            p::Mutation::Archive | p::Mutation::Trash | p::Mutation::Spam | p::Mutation::Move(_)
        ) && std::env::var_os("DAKIA_RULES_FIXTURE").is_none()
        {
            page.rows.retain(|m| m.reference != *r);
        }
        let destination = match change {
            p::Mutation::Archive if gmail => None,
            p::Mutation::Archive => Some("Archive".into()),
            p::Mutation::Trash => Some("Trash".into()),
            p::Mutation::Spam => Some("Junk".into()),
            p::Mutation::Move(f) => Some(f),
            _ => None,
        };
        let mut destination_reference = None;
        if std::env::var_os("DAKIA_RULES_FIXTURE").is_some() {
            if let Some(folder) = &destination {
                for message in page.rows.iter_mut().filter(|m| m.reference == *r) {
                    message.reference.folder = folder.clone();
                    message.identity = p::Identity::Imap(message.reference.clone());
                    destination_reference = Some(message.reference.clone());
                }
            }
        }
        Ok(p::MutationResult {
            destination,
            destination_reference,
            requires_rescan: false,
        })
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        panic!("no IDLE in this fixture")
    }
    async fn submit(
        &self,
        _: &p::mime::PreparedMessage,
        _: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        panic!("no SMTP in this fixture")
    }
    async fn reconcile_sent(&self, _: &str, _: &[u8]) -> p::Result<p::SentOutcome> {
        panic!("no Sent filing in this fixture")
    }
}

/// Dedicated browser fixture: real HTTP/auth/storage/content/mutation handlers,
/// with only the provider replaced. It cannot connect to real mail or send SMTP.
#[tokio::test]
#[ignore = "started only by playwright.backend.config.ts on loopback"]
async fn browser_release_fixture() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let (_dir, mut app, user, a, provider) = controlled().await;
    {
        let mut page = provider.page.lock().await;
        let template = page.rows[0].clone();
        let count = if std::env::var_os("DAKIA_CLEANUP_FIXTURE").is_some()
            && std::env::var_os("DAKIA_RULES_FIXTURE").is_none()
        {
            48
        } else {
            8
        };
        for i in 2..count {
            let mut m = template.clone();
            m.reference.uid = i + 1;
            if i == 7 {
                m.seen = true;
            }
            m.identity = p::Identity::Imap(m.reference.clone());
            m.headers = format!("From: fixture@example.invalid\r\nSubject: Synthetic {i}\r\nMessage-ID: <browser{i}@example.invalid>\r\n\r\n").into_bytes();
            page.rows.push(m);
        }
    }
    app.mail.probe(&app.store, &a).await.unwrap();
    crate::sync::tick(&app, &a).await.unwrap();
    let origin = "http://127.0.0.1:4327";
    if std::env::var_os("DAKIA_FULL_TEXT_FIXTURE").is_some() {
        app.store.index_search_headers(&a).await.unwrap();
        for m in app.store.inbox_rows(&a, None, None).await.unwrap() {
            let raw = provider.raw.lock().await.clone();
            let (text, state) = crate::sync::full_text::extract(&raw);
            app.store
                .put_search_body(
                    &a,
                    &m,
                    &format!("{text}\nbody-only-historical café 東京"),
                    state,
                )
                .await
                .unwrap();
        }
    }
    // Pre-existing successful unsubscribe, entirely local and synthetic.
    let source = app
        .store
        .inbox_rows(&a, None, None)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.subject == "Synthetic 7")
        .unwrap();
    let content: MessageContent = serde_json::from_value(serde_json::json!({
        "message": source, "text": "Read-only sender cleanup preview.", "html": "", "attachments": [], "message_id": null, "references": [], "cc": []
    })).unwrap();
    app.store
        .cache_content(&a, &content, 1024 * 1024)
        .await
        .unwrap();
    app.store
        .put_provider_document(
            &a,
            &format!("unsubscribe-v1:{}", source.id),
            r#"{"kind":"one_click","url":"https://newsletter.example.org/unsubscribe"}"#,
        )
        .await
        .unwrap();
    app.store
        .put_provider_document(
            &a,
            &format!("unsubscribe-attempt:{}", source.id),
            "request_accepted",
        )
        .await
        .unwrap();
    app.config = Arc::new(crate::Config {
        origin: origin.into(),
        issuer: "https://id.example.invalid".into(),
        client_id: "fixture".into(),
        client_secret: "fixture-only".into(),
        web_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/web/dist")
            .to_string_lossy()
            .into(),
    });
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(b"release-browser-fixture")),
        )
        .bind(&user)
        .bind("release-browser-csrf")
        .bind(4102444800_i64)
        .execute(app.store.control())
        .await
        .unwrap();
    let worker_app = app.clone();
    let worker = tokio::spawn(async move {
        loop {
            crate::mutations::tick(&worker_app, &a).await.unwrap();
            tokio::time::sleep(Duration::from_millis(
                if std::env::var_os("DAKIA_CLEANUP_FIXTURE").is_some() {
                    300
                } else {
                    20
                },
            ))
            .await;
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4327")
        .await
        .unwrap();
    println!("Disposable release browser fixture ready at {origin}");
    axum::serve(listener, crate::router(app)).await.unwrap();
    worker.abort();
}
async fn controlled() -> (
    tempfile::TempDir,
    crate::App,
    String,
    AuthorizedAccount,
    Arc<Controlled>,
) {
    let (dir, app, user, a, base) = setup(2, 0).await;
    let page = base
        .scan("INBOX", &p::Search::default(), None, 50)
        .await
        .unwrap();
    base.scans.store(0, Ordering::SeqCst);
    let provider = Arc::new(Controlled {
        base,
        page: Mutex::new(page),
        failure: Mutex::new(None),
        head_calls: AtomicUsize::new(0),
        fetch_calls: AtomicUsize::new(0),
        recovery_calls: AtomicUsize::new(0),
        recovered: Mutex::new(None),
        raw: Mutex::new(MIME.to_vec()),
        mutations: Mutex::new(vec![]),
        stale_inbox: AtomicBool::new(false),
        search_delay: Mutex::new(Duration::ZERO),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    (dir, app, user, a, provider)
}
async fn database(dir: &tempfile::TempDir, a: &AuthorizedAccount) -> sqlx::SqlitePool {
    sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap()
}

#[tokio::test]
async fn rejected_epoch_page_invalidates_old_uids_but_does_not_publish_messages_or_watermark() {
    let (_dir, app, _, a, provider) = controlled().await;
    let old = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    {
        let mut page = provider.page.lock().await;
        page.uidvalidity = 2;
        for m in &mut page.rows {
            m.reference.uidvalidity = 2;
            m.identity = p::Identity::Imap(m.reference.clone());
        }
        page.rows[1].internal_date = "invalid date".into();
    }
    assert!(app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .is_err());
    // Once the server reports a new UIDVALIDITY, old UID references are unsafe
    // even if a header in the replacement page is invalid. Preserve metadata,
    // hide stale references, and never advance the durable arrival checkpoint.
    assert_eq!(
        app.store
            .provider_document(&a, "thread:epoch:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("2")
    );
    assert!(app
        .store
        .inbox_rows(&a, None, None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(app.store.catalogue(&a).await.unwrap().len(), old.rows.len());
    assert!(app
        .store
        .provider_document(&a, "sync:head:INBOX")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn epoch_change_first_valid_page_commits_without_spurious_inbox_error() {
    let (_dir, app, _, a, provider) = controlled().await;
    crate::sync::tick(&app, &a).await.unwrap();
    app.store.reset_scan(&a, "INBOX").await.unwrap();
    // Select a historical turn, independently of the arrival-head scheduler.
    app.store
        .put_provider_document(&a, "sync:folders", "{}")
        .await
        .unwrap();
    {
        let mut page = provider.page.lock().await;
        page.uidvalidity = 2;
        for m in &mut page.rows {
            m.reference.uidvalidity = 2;
            m.identity = p::Identity::Imap(m.reference.clone());
        }
    }
    assert!(
        crate::sync::tick(&app, &a).await.is_ok(),
        "a valid UIDVALIDITY replacement page must commit into its new generation"
    );
    assert_eq!(
        app.store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Complete
    );
}

macro_rules! rejected_page {
    ($name:ident, $alter:expr) => {
        #[tokio::test]
        async fn $name() {
            let (_dir, app, _, a, provider) = controlled().await;
            ($alter)(&mut *provider.page.lock().await);
            assert!(app
                .mail
                .snapshot_page(&app.store, &a, "INBOX", None)
                .await
                .is_err());
            assert!(app.store.catalogue(&a).await.unwrap().is_empty());
        }
    };
}
rejected_page!(
    foreign_reference_rejects_entire_page,
    |page: &mut p::Page| page.rows[1].reference.account_id = "foreign".into()
);
rejected_page!(wrong_folder_rejects_entire_page, |page: &mut p::Page| {
    page.rows[1].reference.folder = "Sent".into()
});
rejected_page!(zero_uid_rejects_entire_page, |page: &mut p::Page| page
    .rows[1]
    .reference
    .uid =
    0);
rejected_page!(
    foreign_identity_rejects_entire_page,
    |page: &mut p::Page| page.rows[1].identity = p::Identity::Gmail {
        account_id: "foreign".into(),
        message_id: 1
    }
);
rejected_page!(oversized_page_rejects_entire_page, |page: &mut p::Page| {
    page.rows = vec![page.rows[0].clone(); 201]
});

#[tokio::test]
async fn checkpoint_write_failure_replays_ingestion_without_duplicate_messages() {
    let (dir, app, _, a, provider) = controlled().await;
    let db = database(&dir, &a).await;
    sqlx::query("CREATE TRIGGER fail_head BEFORE INSERT ON provider_documents WHEN NEW.key='sync:head:INBOX' BEGIN SELECT RAISE(ABORT,'fixture failure'); END").execute(&db).await.unwrap();
    assert!(app.mail.check_head(&app.store, &a, "INBOX").await.is_err());
    assert!(app
        .store
        .provider_document(&a, "sync:head:INBOX")
        .await
        .unwrap()
        .is_none());
    let before = app.store.catalogue(&a).await.unwrap();
    assert_eq!(before.len(), 2);
    sqlx::query("DROP TRIGGER fail_head")
        .execute(&db)
        .await
        .unwrap();
    app.store
        .put_provider_document(&a, "sync:head-at:INBOX", "0")
        .await
        .unwrap();
    assert!(app.mail.check_head(&app.store, &a, "INBOX").await.unwrap());
    assert_eq!(app.store.catalogue(&a).await.unwrap(), before);
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 2);
    db.close().await;
}

#[tokio::test]
async fn head_budget_coalesces_success_and_failure_without_phantom_notifications() {
    let (_dir, app, _, a, provider) = controlled().await;
    assert!(app.mail.check_head(&app.store, &a, "INBOX").await.unwrap());
    for _ in 0..30 {
        assert!(!app.mail.check_head(&app.store, &a, "INBOX").await.unwrap());
    }
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 1);
    assert!(app.store.next_push(&a).await.unwrap().is_none());
    let checkpoint = app
        .store
        .provider_document(&a, "sync:head:INBOX")
        .await
        .unwrap();
    app.store
        .put_provider_document(&a, "sync:head-at:INBOX", "0")
        .await
        .unwrap();
    *provider.failure.lock().await = Some(p::ErrorKind::Retryable);
    assert!(app.mail.check_head(&app.store, &a, "INBOX").await.is_err());
    for _ in 0..30 {
        assert!(!app.mail.check_head(&app.store, &a, "INBOX").await.unwrap());
    }
    assert_eq!(provider.head_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        app.store
            .provider_document(&a, "sync:head:INBOX")
            .await
            .unwrap(),
        checkpoint
    );
}

#[tokio::test]
async fn unchanged_inbox_refresh_uses_cached_bodies_and_does_not_emit_events() {
    let (dir, app, _, a, provider) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let id = &page.rows[0].id;
    app.mail.content(&app.store, &a, id).await.unwrap();
    let db = database(&dir, &a).await;
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&db)
        .await
        .unwrap();
    for _ in 0..5 {
        app.mail
            .snapshot_page(&app.store, &a, "INBOX", None)
            .await
            .unwrap();
        app.mail.content(&app.store, &a, id).await.unwrap();
    }
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
            .fetch_one(&db)
            .await
            .unwrap(),
        events
    );
    db.close().await;
}

#[tokio::test]
async fn simultaneous_opens_share_one_body_fetch_and_attachment_identity() {
    let (_dir, app, _, a, provider) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let id = &page.rows[0].id;
    let (left, right) = tokio::join!(
        app.mail.content(&app.store, &a, id),
        app.mail.content(&app.store, &a, id)
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert_eq!(
        provider.fetch_calls.load(Ordering::SeqCst),
        1,
        "reader/background callers must share an in-flight body fetch"
    );
    assert_eq!(left.attachments, right.attachments);
}

#[tokio::test]
async fn concurrent_content_responses_keep_the_same_attachment_ids() {
    let (_dir, app, _, a, _) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let id = &page.rows[0].id;
    let (left, right) = tokio::join!(
        app.mail.content(&app.store, &a, id),
        app.mail.content(&app.store, &a, id)
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert_eq!(
        left.attachments, right.attachments,
        "an attachment must have one identity across simultaneous readers"
    );
}

#[tokio::test]
async fn download_after_open_reuses_already_fetched_attachment_bytes() {
    let (_dir, app, _, a, provider) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let body = app
        .mail
        .content(&app.store, &a, &page.rows[0].id)
        .await
        .unwrap();
    for part in &body.attachments {
        let bytes = app
            .mail
            .received_attachment(&app.store, &a, &part.id)
            .await
            .unwrap();
        assert_eq!(bytes, b"attachment bytes");
    }
    assert_eq!(
        provider.fetch_calls.load(Ordering::SeqCst),
        1,
        "opening and downloading the same attachment should not download the entire message twice"
    );
}

#[tokio::test]
async fn received_download_survives_provider_outage_and_preserves_ids_after_eviction() {
    let (_dir, app, _, a, provider) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let body = app
        .mail
        .content(&app.store, &a, &page.rows[0].id)
        .await
        .unwrap();
    *provider.failure.lock().await = Some(p::ErrorKind::Retryable);
    let part = &body.attachments[0];
    assert_eq!(
        app.mail
            .received_attachment(&app.store, &a, &part.id)
            .await
            .unwrap(),
        b"attachment bytes"
    );
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 1);
    app.store.evict_received(&a).await.unwrap();
    assert!(app
        .mail
        .received_attachment(&app.store, &a, &part.id)
        .await
        .is_err());
    *provider.failure.lock().await = None;
    assert_eq!(
        app.mail
            .received_attachment(&app.store, &a, &part.id)
            .await
            .unwrap(),
        b"attachment bytes"
    );
    let restored = app
        .mail
        .content(&app.store, &a, &page.rows[0].id)
        .await
        .unwrap();
    assert_eq!(restored.attachments, body.attachments);
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn prewrite_action_failure_backs_off_then_completes_without_browser_replay() {
    let (dir, app, _, a, provider) = controlled().await;
    let m = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows
        .remove(0);
    app.store
        .operation(
            &a,
            Operation {
                id: "retry".into(),
                account_id: a.id().into(),
                target_id: m.id,
                expected_revision: m.revision,
                change: Change::Archive,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    *provider.failure.lock().await = Some(p::ErrorKind::Retryable);
    crate::mutations::tick(&app, &a).await.unwrap();
    app.store.recover_mutations(&a).await.unwrap();
    assert_eq!(
        app.store
            .operation_status(&a, "retry")
            .await
            .unwrap()
            .status,
        OperationStatus::Pending
    );
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(
        provider.mutations.lock().await.len(),
        1,
        "backoff must prevent busy retry"
    );
    let db = database(&dir, &a).await;
    sqlx::query("UPDATE outbox SET lease_until='2000-01-01T00:00:00.000Z' WHERE target_id='retry'")
        .execute(&db)
        .await
        .unwrap();
    *provider.failure.lock().await = None;
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store
            .operation_status(&a, "retry")
            .await
            .unwrap()
            .status,
        OperationStatus::Applied
    );
    assert!(app
        .store
        .provider_document(&a, "mutation-retry:retry")
        .await
        .unwrap()
        .is_none());
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(provider.mutations.lock().await.len(), 2);
    db.close().await;
}

#[tokio::test]
async fn uncertain_action_never_enters_the_safe_retry_queue() {
    let (_dir, app, _, a, provider) = controlled().await;
    let m = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows
        .remove(0);
    app.store
        .operation(
            &a,
            Operation {
                id: "uncertain".into(),
                account_id: a.id().into(),
                target_id: m.id,
                expected_revision: m.revision,
                change: Change::Trash,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    *provider.failure.lock().await = Some(p::ErrorKind::Uncertain);
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store
            .operation_status(&a, "uncertain")
            .await
            .unwrap()
            .status,
        OperationStatus::Failed
    );
    assert!(app
        .store
        .provider_document(&a, "mutation-retry:uncertain")
        .await
        .unwrap()
        .is_none());
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(provider.mutations.lock().await.len(), 1);
}

macro_rules! user_mutation {
    ($name:ident, $change:expr, $destination:expr) => {
        #[tokio::test]
        async fn $name() {
            let (_dir, app, _, a, provider) = controlled().await;
            let m = app
                .mail
                .snapshot_page(&app.store, &a, "INBOX", None)
                .await
                .unwrap()
                .rows
                .remove(0);
            app.store
                .operation(
                    &a,
                    Operation {
                        id: "action".into(),
                        account_id: a.id().into(),
                        target_id: m.id.clone(),
                        expected_revision: m.revision,
                        change: $change,
                        status: OperationStatus::Pending,
                        revision: 0,
                    },
                )
                .await
                .unwrap();
            crate::mutations::tick(&app, &a).await.unwrap();
            assert_eq!(
                app.store
                    .operation_status(&a, "action")
                    .await
                    .unwrap()
                    .status,
                OperationStatus::Applied
            );
            assert_eq!(
                app.store.message(&a, &m.id).await.unwrap().folder_ids,
                vec![$destination.to_string()]
            );
            crate::mutations::tick(&app, &a).await.unwrap();
            assert_eq!(
                provider.mutations.lock().await.len(),
                1,
                "a completed operation must not be replayed"
            );
        }
    };
}
user_mutation!(
    archive_runs_once_and_persists_destination,
    Change::Archive,
    "Archive"
);
user_mutation!(
    trash_runs_once_and_persists_destination,
    Change::Trash,
    "Trash"
);
user_mutation!(
    spam_runs_once_and_persists_destination,
    Change::Spam,
    "Junk"
);
user_mutation!(
    move_runs_once_and_persists_destination,
    Change::Move {
        folder_id: "Projects".into()
    },
    "Projects"
);

#[tokio::test]
async fn verified_gmail_archive_does_not_reuse_stale_inbox_uid_for_next_action() {
    let (_dir, app, _, a, provider) = controlled().await;
    {
        let mut page = provider.page.lock().await;
        for m in &mut page.rows {
            m.identity = p::Identity::Gmail {
                account_id: a.id().into(),
                message_id: u64::from(m.reference.uid),
            };
            m.gmail_thread_id = Some(99);
            m.labels = vec!["\\Inbox".into(), "All Mail".into()];
        }
    }
    let m = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows
        .remove(0);
    let alternative = p::MessageRef {
        account_id: a.id().into(),
        folder: "All Mail".into(),
        uidvalidity: 7,
        uid: 800,
    };
    app.store
        .put_provider_document(
            &a,
            &format!("member-reference:{}:All Mail", m.id),
            &serde_json::to_string(&alternative).unwrap(),
        )
        .await
        .unwrap();
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
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store
            .operation_status(&a, "archive")
            .await
            .unwrap()
            .status,
        OperationStatus::Applied
    );
    provider.stale_inbox.store(true, Ordering::SeqCst); // archive removed this membership
    let m = app.store.message(&a, &m.id).await.unwrap();
    assert!(m.folder_ids.contains(&"All Mail".into()));
    assert!(!m.folder_ids.contains(&"INBOX".into()));
    app.store
        .operation(
            &a,
            Operation {
                id: "star-after-archive".into(),
                account_id: a.id().into(),
                target_id: m.id,
                expected_revision: m.revision,
                change: Change::Star { value: true },
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store
            .operation_status(&a, "star-after-archive")
            .await
            .unwrap()
            .status,
        OperationStatus::Applied,
        "known surviving Gmail membership should keep subsequent actions usable"
    );
}

#[tokio::test]
async fn encoded_attachment_above_old_30mib_limit_opens_and_downloads_exact_bytes_through_handler()
{
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let (_dir, app, user, a, provider) = controlled().await;
    let bytes = vec![0x97; 23 * 1024 * 1024];
    let expected = Sha256::digest(&bytes);
    let raw=format!("MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nFixture\r\n--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=large.bin\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n--x--\r\n",base64::engine::general_purpose::STANDARD.encode(&bytes)).into_bytes();
    drop(bytes);
    assert!(raw.len() > 30 * 1024 * 1024);
    *provider.raw.lock().await = raw;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let id = &page.rows[0].id;
    let content = app.mail.content(&app.store, &a, id).await.unwrap();
    assert_eq!(content.attachments.len(), 1);
    let part = &content.attachments[0];
    assert_eq!(part.size_bytes, 23 * 1024 * 1024);
    let response = crate::download(
        State(app.clone()),
        Extension(crate::Identity {
            user: user.clone(),
            csrf: "fixture".into(),
            token_hash: "fixture".into(),
        }),
        axum::extract::Path((a.id().into(), part.id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert!(response.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("large.bin"));
    let downloaded = axum::body::to_bytes(response.into_body(), 40 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(Sha256::digest(&downloaded), expected);
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 1);
    // Durable account authorization still precedes content lookup and download.
    let other = app.store.user("fixture", "other-owner").await.unwrap();
    assert!(crate::download(
        State(app.clone()),
        Extension(crate::Identity {
            user: other,
            csrf: "fixture".into(),
            token_hash: "fixture".into()
        }),
        axum::extract::Path((a.id().into(), part.id.clone()))
    )
    .await
    .is_err());
    // The new limit remains finite, with a permanent, explicit size error.
    app.store.evict_received(&a).await.unwrap();
    *provider.raw.lock().await = vec![0; 40 * 1024 * 1024 + 1];
    let error = app.mail.raw(&app.store, &a, id).await.unwrap_err();
    assert_eq!(
        (error.0, error.1),
        (StatusCode::PAYLOAD_TOO_LARGE, "message_too_large")
    );
}
