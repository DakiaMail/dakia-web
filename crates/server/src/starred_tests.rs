use super::*;
use crate::sync::starred::tick;

struct Stars {
    account: String,
    gmail: bool,
    offline: AtomicBool,
    epoch: AtomicUsize,
    count: AtomicUsize,
    fetches: AtomicUsize,
    visits: Mutex<Vec<(String, u32)>>,
}
#[async_trait]
impl p::MailProvider for Stars {
    fn account_id(&self) -> &str {
        &self.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        unreachable!()
    }
    async fn scan(
        &self,
        folder: &str,
        query: &p::Search,
        token: Option<&p::PageToken>,
        limit: usize,
    ) -> p::Result<p::Page> {
        assert_eq!(query.starred, Some(true));
        assert_eq!(limit, 50);
        if self.offline.load(Ordering::SeqCst) {
            return Err(p::error(p::ErrorKind::Retryable, "private fixture error"));
        }
        let epoch = self.epoch.load(Ordering::SeqCst) as u32;
        if token.is_some_and(|t| t.uidvalidity != epoch) {
            return Err(p::error(p::ErrorKind::StaleReference, "fixture"));
        }
        let start = token.map_or(0, |t| t.before_uid);
        self.visits.lock().await.push((folder.into(), start));
        let count = self.count.load(Ordering::SeqCst) as u32;
        let end = (start + limit as u32).min(count);
        let rows = (start..end)
            .map(|i| {
                let reference = p::MessageRef {
                    account_id: self.account.clone(),
                    folder: folder.into(),
                    uidvalidity: epoch,
                    uid: i + 1,
                };
                let identity = if self.gmail {
                    p::Identity::Gmail {
                        account_id: self.account.clone(),
                        message_id: u64::from(i + 1),
                    }
                } else {
                    p::Identity::Imap(reference.clone())
                };
                p::Metadata {
                    reference,
                    identity,
                    gmail_thread_id: None,
                    labels: vec![],
                    seen: false,
                    starred: true,
                    size: MIME.len() as u32,
                    internal_date: "29-Oct-2026 00:00:00 +0000".into(),
                    headers: MIME.to_vec(),
                }
            })
            .collect();
        Ok(p::Page {
            rows,
            uidvalidity: epoch,
            full_coverage: !self.gmail,
            next: (end < count).then(|| p::PageToken {
                account_id: self.account.clone(),
                folder: folder.into(),
                uidvalidity: epoch,
                upper_uid: count,
                before_uid: end,
                query: query.clone(),
                modseq: None,
            }),
        })
    }
    async fn search(
        &self,
        _: &str,
        _: &p::Search,
        _: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        unreachable!()
    }
    async fn fetch(&self, r: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        assert_eq!(r.account_id, self.account);
        self.fetches.fetch_add(1, Ordering::SeqCst);
        if self.offline.load(Ordering::SeqCst) {
            return Err(p::error(p::ErrorKind::Retryable, "private fixture error"));
        }
        Ok(MIME.to_vec())
    }
    async fn mutate(&self, _: &p::MessageRef, _: p::Mutation) -> p::Result<p::MutationResult> {
        panic!("precache must never mutate flags")
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        unreachable!()
    }
    async fn submit(
        &self,
        _: &p::mime::PreparedMessage,
        _: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        panic!("no mail sends")
    }
    async fn reconcile_sent(&self, _: &str, _: &[u8]) -> p::Result<p::SentOutcome> {
        panic!("no filing")
    }
}
async fn install(
    app: &crate::App,
    a: &dakia_storage::AuthorizedAccount,
    gmail: bool,
    count: usize,
) -> Arc<Stars> {
    let provider = Arc::new(Stars {
        account: a.id().into(),
        gmail,
        offline: AtomicBool::new(false),
        epoch: AtomicUsize::new(1),
        count: AtomicUsize::new(count),
        fetches: AtomicUsize::new(0),
        visits: Mutex::default(),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    app.store.put_provider_document(a, "sync:folders", &serde_json::json!({"folders":[{"name":"INBOX","selectable":true,"attributes":[]},{"name":"Archive","selectable":true,"attributes":[]}],"index":0,"refreshed":0}).to_string()).await.unwrap();
    provider
}
async fn resume(app: &crate::App, a: &dakia_storage::AuthorizedAccount) {
    let raw = app
        .store
        .provider_document(a, "sync:starred")
        .await
        .unwrap()
        .unwrap();
    let mut state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    state["body_due"] = 0.into();
    for scan in state["scans"].as_object_mut().unwrap().values_mut() {
        scan["due"] = 0.into();
    }
    app.store
        .put_provider_document(a, "sync:starred", &state.to_string())
        .await
        .unwrap();
}
async fn rows(app: &crate::App, a: &dakia_storage::AuthorizedAccount) -> Vec<Message> {
    let mut all = vec![];
    let mut after = String::new();
    loop {
        let page = app.store.starred_cache_rows(a, &after).await.unwrap();
        if page.is_empty() {
            return all;
        }
        after = page.last().unwrap().id.clone();
        all.extend(page);
    }
}
// Advance a completed body-pass cooldown without wall-clock sleeps or forcing
// provider scans to restart. New random IDs may precede the previous keyset.
async fn body_cycle(app: &crate::App, a: &dakia_storage::AuthorizedAccount) {
    let raw = app
        .store
        .provider_document(a, "sync:starred")
        .await
        .unwrap()
        .unwrap();
    let mut state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    state["body_due"] = 0.into();
    state["body_after"] = "".into();
    app.store
        .put_provider_document(a, "sync:starred", &state.to_string())
        .await
        .unwrap();
    for _ in 0..60 {
        tick(app, a).await.unwrap();
    }
}
#[tokio::test]
async fn all_starred_pages_bodies_accounts_restart_and_offline_open() {
    let (_dir, mut app, user, a, _) = setup(0, 0).await;
    let input = app.store.credential(&a).await.unwrap();
    let b = app.store.connect(&user, &input).await.unwrap();
    let b = app.store.authorize(&user, &b.id).await.unwrap();
    let one = install(&app, &a, true, 73).await;
    let two = install(&app, &b, false, 53).await;
    for _ in 0..100 {
        for (a, p) in [(&a, &one), (&b, &two)] {
            let before = p.fetches.load(Ordering::SeqCst);
            tick(&app, a).await.unwrap();
            assert!(p.fetches.load(Ordering::SeqCst) <= before + 2);
        }
    }
    body_cycle(&app, &a).await;
    body_cycle(&app, &b).await;
    assert_eq!(one.visits.lock().await.len(), 4);
    assert_eq!(two.visits.lock().await.len(), 4);
    assert!(one.visits.lock().await.contains(&("Archive".into(), 50)));
    assert_eq!(rows(&app, &a).await.len(), 73); // Gmail stable IDs dedupe labels
    assert_eq!(rows(&app, &b).await.len(), 106); // IMAP identities remain folder-local
    assert_eq!(one.fetches.load(Ordering::SeqCst), 73);
    assert_eq!(two.fetches.load(Ordering::SeqCst), 106);
    one.offline.store(true, Ordering::SeqCst);
    two.offline.store(true, Ordering::SeqCst);
    // Restart runtime, reuse durable cursors and bodies rather than fetch again.
    app.mail = MailRuntime::default();
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), one.clone());
    app.mail
        .providers
        .lock()
        .await
        .insert(b.id().into(), two.clone());
    for a in [&a, &b] {
        resume(&app, a).await;
        tick(&app, a).await.unwrap();
        for m in rows(&app, a).await {
            let c = app.mail.content(&app.store, a, &m.id).await.unwrap();
            assert_eq!(c.text.trim(), "Synthetic only");
            assert!(c.message.unread && c.message.starred);
            assert_eq!(c.attachments.len(), 1);
        }
        let state = app
            .store
            .provider_document(a, "sync:starred")
            .await
            .unwrap()
            .unwrap();
        assert!(state.contains("provider_unavailable"));
        assert!(!state.contains("private fixture"));
    }
    assert_eq!(one.fetches.load(Ordering::SeqCst), 73);
    assert_eq!(two.fetches.load(Ordering::SeqCst), 106);
    let id = rows(&app, &a).await[0].id.clone();
    assert!(app.store.cached_content(&b, &id).await.unwrap().is_none());
    // Real Starred navigation is local and paginates every cached account even
    // while the mail provider is offline. No private browser persistence added.
    let mut request =
        serde_json::json!({"q":"is:starred", "accounts":format!("{},{}",a.id(),b.id())});
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let page = search(&app, &user, request.clone()).await.unwrap();
        assert!(page
            .coverage
            .iter()
            .all(|c| c.state == CoverageState::Error));
        for m in page.rows {
            assert!(seen.insert((m.account_id, m.id)));
        }
        match page.continuation {
            Some(token) => request["continuation"] = token.into(),
            None => break,
        }
    }
    assert_eq!(seen.len(), 179);
}

#[tokio::test]
async fn transitions_eviction_missing_blobs_and_epoch_invalidation() {
    let (dir, app, _, a, _) = setup(0, 0).await;
    let provider = install(&app, &a, false, 2).await;
    for _ in 0..5 {
        tick(&app, &a).await.unwrap();
    }
    let all = rows(&app, &a).await;
    body_cycle(&app, &a).await;
    assert_eq!(all.len(), 4);
    // Unstar applies immediately to the next body walk and cached flags.
    let mut unstar = all[0].clone();
    unstar.starred = false;
    app.store.upsert_message(&a, unstar.clone()).await.unwrap();
    assert_eq!(rows(&app, &a).await.len(), 3);
    assert!(
        !app.store
            .cached_content(&a, &unstar.id)
            .await
            .unwrap()
            .unwrap()
            .message
            .starred
    );
    // Explicit eviction is respected, and warming repairs it on the next pass.
    app.store.evict_received(&a).await.unwrap();
    body_cycle(&app, &a).await;
    for m in rows(&app, &a).await {
        assert!(app.store.cached_content(&a, &m.id).await.unwrap().is_some());
    }
    // Missing files also recover without requiring the owner to open the mail.
    let cache = dir.path().join("cache").join(a.id());
    for file in std::fs::read_dir(cache).unwrap() {
        std::fs::remove_file(file.unwrap().path()).unwrap();
    }
    body_cycle(&app, &a).await;
    for m in rows(&app, &a).await {
        assert!(app.store.cached_content(&a, &m.id).await.unwrap().is_some());
    }
    // UIDVALIDITY changes exclude obsolete generic IMAP identities from warming.
    app.store
        .put_provider_document(&a, "thread:epoch:INBOX", "2")
        .await
        .unwrap();
    assert!(rows(&app, &a)
        .await
        .iter()
        .all(|m| !m.folder_ids.contains(&"INBOX".into())));
    // Provider recovery and new stars are discovered by another full pagination pass.
    provider.count.store(3, Ordering::SeqCst);
    resume(&app, &a).await;
    for _ in 0..6 {
        tick(&app, &a).await.unwrap();
    }
    assert_eq!(rows(&app, &a).await.len(), 6);
    body_cycle(&app, &a).await;
    for m in rows(&app, &a).await {
        assert!(app.store.cached_content(&a, &m.id).await.unwrap().is_some());
    }
}

#[tokio::test]
async fn verified_complete_snapshots_suppress_starred_work_across_restart_and_revoke_on_change() {
    let (dir, mut app, _user, a, _) = setup(0, 0).await;
    let provider = install(&app, &a, false, 1).await;
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    for folder in ["INBOX", "Archive"] {
        let (generation, _) = app.store.scan_state(&a, folder).await.unwrap().unwrap();
        app.store
            .put_provider_document(&a, &format!("thread:epoch:{folder}"), "1")
            .await
            .unwrap();
        sqlx::query("UPDATE folder_scans SET completed_at=?,uidvalidity=1,coverage_limited=0 WHERE folder=?")
            .bind(chrono::Utc::now().timestamp()).bind(folder).execute(&db).await.unwrap();
        assert!(!app
            .store
            .verified_snapshot_current(&a, folder)
            .await
            .unwrap());
        app.store
            .confirm_unchanged_scan(&a, folder, generation)
            .await
            .unwrap();
        assert!(app
            .store
            .verified_snapshot_current(&a, folder)
            .await
            .unwrap());
    }
    // Four hours of five-minute eligibility windows, without provider calls.
    for _ in 0..48 {
        tick(&app, &a).await.unwrap();
        resume(&app, &a).await;
    }
    assert!(provider.visits.lock().await.is_empty());
    app.mail = MailRuntime::default();
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    tick(&app, &a).await.unwrap();
    assert!(provider.visits.lock().await.is_empty());
    app.store
        .put_provider_document(&a, "thread:epoch:INBOX", "2")
        .await
        .unwrap();
    assert!(!app
        .store
        .verified_snapshot_current(&a, "INBOX")
        .await
        .unwrap());
    provider.epoch.store(2, Ordering::SeqCst);
    resume(&app, &a).await;
    tick(&app, &a).await.unwrap();
    assert_eq!(provider.visits.lock().await.len(), 1);
    sqlx::query("UPDATE folder_scans SET coverage_limited=1 WHERE folder='Archive'")
        .execute(&db)
        .await
        .unwrap();
    assert!(!app
        .store
        .verified_snapshot_current(&a, "Archive")
        .await
        .unwrap());
    sqlx::query("UPDATE folder_scans SET coverage_limited=0 WHERE folder='Archive'")
        .execute(&db)
        .await
        .unwrap();
    let proof = app
        .store
        .provider_document(&a, "sync:verified:Archive")
        .await
        .unwrap()
        .unwrap();
    let mut expired: serde_json::Value = serde_json::from_str(&proof).unwrap();
    expired["verified_at"] = (chrono::Utc::now().timestamp() - 301).into();
    app.store
        .put_provider_document(&a, "sync:verified:Archive", &expired.to_string())
        .await
        .unwrap();
    assert!(!app
        .store
        .verified_snapshot_current(&a, "Archive")
        .await
        .unwrap());
    app.store
        .put_provider_document(&a, "sync:verified:Archive", &proof)
        .await
        .unwrap();
    app.store.reset_scan(&a, "Archive").await.unwrap();
    assert!(!app
        .store
        .verified_snapshot_current(&a, "Archive")
        .await
        .unwrap());
}
