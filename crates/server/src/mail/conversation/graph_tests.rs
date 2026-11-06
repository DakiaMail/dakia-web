use super::*;

#[tokio::test]
async fn indexed_snapshots_are_cache_first_isolated_live_and_honest_about_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let user = store.user("fixture", "graph").await.unwrap();
    let a = account(&store, &user).await;
    let b = account(&store, &user).await;
    let runtime = MailRuntime::default();
    let provider = Arc::new(Provider {
        account: a.id().into(),
        gmail: false,
        delay_ms: 0,
        fail: AtomicBool::new(false),
        scans: AtomicUsize::new(0),
        rows: vec![
            ("INBOX".into(), raw("root", "", 26), 1),
            (
                "Sent Items".into(),
                raw("reply", "In-Reply-To: <root@example.invalid>\r\n", 27),
                2,
            ),
            (
                "Archive".into(),
                raw("last", "References: <reply@example.invalid>\r\n", 28),
                3,
            ),
        ],
    });
    runtime
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    store
        .put_provider_document(
            &a,
            "probe",
            &serde_json::to_string(&provider.probe().await.unwrap()).unwrap(),
        )
        .await
        .unwrap();
    runtime.folders(&store, &a).await.unwrap();
    let mut seed = String::new();
    let mut last = String::new();
    for f in folders() {
        let page = runtime
            .page(&store, &a, &f.name, p::Search::default(), None)
            .await
            .unwrap();
        if f.name == "INBOX" {
            seed = page.rows[0].id.clone();
        }
        if f.name == "Archive" {
            last = page.rows[0].id.clone();
        }
    }
    provider.scans.store(0, Ordering::SeqCst);
    let known = runtime.conversation(&store, &a, &seed, None).await.unwrap();
    assert_eq!(known.rows.len(), 3);
    assert!(!known.complete);
    assert!(known.continuation.is_some());
    assert_eq!(provider.scans.load(Ordering::SeqCst), 0);
    assert!(store
        .thread_neighbors(&b, &["rfc:root@example.invalid".into()])
        .await
        .unwrap()
        .is_empty());
    // Certify every folder only through actual complete reconciliation.
    for f in folders() {
        let (generation, _) = store.scan_state(&a, &f.name).await.unwrap().unwrap();
        let page = runtime
            .page(&store, &a, &f.name, p::Search::default(), None)
            .await
            .unwrap();
        store
            .record_scan(&a, &f.name, generation, &page)
            .await
            .unwrap();
    }
    provider.scans.store(0, Ordering::SeqCst);
    let reopened = MailRuntime::default(); // no provider available; reads survive restart/offline
    let warm = reopened
        .conversation(&store, &a, &seed, None)
        .await
        .unwrap();
    assert_eq!(warm.rows.len(), 3);
    assert!(warm.complete);
    assert!(warm.continuation.is_none());
    assert_eq!(provider.scans.load(Ordering::SeqCst), 0);
    // Current message rows, not stale serialized conversation snapshots.
    let mut changed = store.message(&a, &last).await.unwrap();
    changed.unread = false;
    changed.starred = true;
    changed.category = Some("people".into());
    store.upsert_message(&a, changed).await.unwrap();
    let warm = reopened
        .conversation(&store, &a, &seed, None)
        .await
        .unwrap();
    let changed = warm.rows.iter().find(|m| m.id == last).unwrap();
    assert!(!changed.unread && changed.starred);
    assert_eq!(changed.category.as_deref(), Some("people"));
    // Deleted memberships cannot resurrect retained historical headers.
    let mut deleted = store.message(&a, &last).await.unwrap();
    deleted.folder_ids.clear();
    store.upsert_message(&a, deleted).await.unwrap();
    assert_eq!(
        reopened
            .conversation(&store, &a, &seed, None)
            .await
            .unwrap()
            .rows
            .len(),
        2
    );
    // A UIDVALIDITY reset rejects identities from the old epoch immediately.
    store
        .put_provider_document(&a, "thread:epoch:Sent Items", "2")
        .await
        .unwrap();
    assert_eq!(
        reopened
            .conversation(&store, &a, &seed, None)
            .await
            .unwrap()
            .rows
            .len(),
        1
    );
    store
        .put_provider_document(&a, "thread:epoch:Sent Items", "1")
        .await
        .unwrap();
    // Removed/renamed folders are excluded even before old scan rows are pruned.
    store
        .put_provider_document(
            &a,
            "thread:folders",
            &serde_json::json!({"names":["INBOX"],"at":chrono::Utc::now().timestamp()}).to_string(),
        )
        .await
        .unwrap();
    assert_eq!(
        reopened
            .conversation(&store, &a, &seed, None)
            .await
            .unwrap()
            .rows
            .len(),
        1
    );
    // Expired topology/coverage is usable known content, never a complete mailbox.
    store
        .put_provider_document(
            &a,
            "thread:folders",
            r#"{"names":["INBOX","Sent Items"],"at":0}"#,
        )
        .await
        .unwrap();
    let stale = reopened
        .conversation(&store, &a, &seed, None)
        .await
        .unwrap();
    assert_eq!(stale.rows.len(), 2);
    assert!(!stale.complete && stale.continuation.is_some());
}
