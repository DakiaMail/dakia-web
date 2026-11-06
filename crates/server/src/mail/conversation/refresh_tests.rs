use super::*;

#[tokio::test]
async fn snapshot_bursts_share_verification_and_discover_unseen_replies() {
    for gmail in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), [42; 32]).await.unwrap();
        let user = store.user("fixture", "refresh").await.unwrap();
        let a = account(&store, &user).await;
        let b = account(&store, &user).await;
        let runtime = MailRuntime::default();
        let provider = Arc::new(Provider {
            account: a.id().into(),
            gmail,
            delay_ms: 5,
            fail: AtomicBool::new(false),
            scans: AtomicUsize::new(0),
            rows: vec![
                ("INBOX".into(), raw("root", "", 26), 1),
                (
                    "Sent Items".into(),
                    raw("reply", "References: <root@example.invalid>\r\n", 27),
                    2,
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
        let seed = runtime
            .page(&store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap()
            .rows[0]
            .id
            .clone();
        provider.scans.store(0, Ordering::SeqCst);
        let initial = runtime.conversation(&store, &a, &seed, None).await.unwrap();
        let shared = runtime.conversation(&store, &a, &seed, None).await.unwrap();
        assert_eq!(initial.continuation, shared.continuation);
        let terminal_token = initial.continuation.clone().unwrap();
        assert_eq!(provider.scans.load(Ordering::SeqCst), 0);
        let result = finish(&runtime, &store, &a, &seed, initial).await;
        assert_eq!(
            result.rows.len(),
            2,
            "previously unknown reply must be discovered"
        );
        let discovery_scans = provider.scans.load(Ordering::SeqCst);
        assert_eq!(
            discovery_scans,
            if gmail { 8 } else { 16 },
            "one bounded initial pass plus final catch-up"
        );
        let revision = store.thread_revision(&a).await.unwrap();
        for i in 0..200 {
            let mut m = store.message(&a, &seed).await.unwrap();
            m.starred = i % 2 == 0;
            m.unread = i % 2 == 1;
            m.category = Some(format!("fixture-{}", i % 3));
            store.upsert_message(&a, m).await.unwrap();
            let snapshot = runtime
                .conversation_read(&store, &a, &seed, None, true, false)
                .await
                .unwrap();
            assert_eq!(snapshot.rows.len(), 2);
            assert!(snapshot.continuation.is_none());
            assert_eq!(
                snapshot.rows.iter().find(|m| m.id == seed).unwrap().starred,
                i % 2 == 0
            );
        }
        assert_eq!(store.thread_revision(&a).await.unwrap(), revision);
        assert_eq!(
            provider.scans.load(Ordering::SeqCst),
            discovery_scans,
            "200 local API reads must do zero provider scans"
        );
        assert!(runtime
            .conversation_read(&store, &b, &seed, None, true, false)
            .await
            .is_err());
        let warm = runtime.conversation(&store, &a, &seed, None).await.unwrap();
        assert!(warm.complete && warm.continuation.is_none());
        // Explicit refresh bypasses a valid proof and the shared cooldown.
        let explicit = runtime
            .conversation_read(&store, &a, &seed, None, false, true)
            .await
            .unwrap();
        assert!(explicit.continuation.is_some());
        finish(&runtime, &store, &a, &seed, explicit).await;
        let explicit_scans = provider.scans.load(Ordering::SeqCst) - discovery_scans;
        assert_eq!(explicit_scans, if gmail { 4 } else { 8 });
        store
            .put_provider_document(&a, "thread:epoch:Empty", "2")
            .await
            .unwrap();
        let before = provider.scans.load(Ordering::SeqCst);
        for _ in 0..20 {
            let dirty = runtime.conversation(&store, &a, &seed, None).await.unwrap();
            assert!(
                !dirty.complete && dirty.continuation.is_none(),
                "dirty cooldown results are partial"
            );
        }
        let replay = runtime
            .conversation(&store, &a, &seed, Some(&terminal_token))
            .await
            .unwrap();
        assert!(
            !replay.complete && replay.continuation.is_none(),
            "terminal replay cannot renew proof"
        );
        assert!(runtime
            .conversation_read(&store, &a, &seed, Some(&terminal_token), true, false)
            .await
            .is_err());
        assert_eq!(provider.scans.load(Ordering::SeqCst), before);
        for (_, shared) in runtime.conversations.lock().await.values() {
            shared.lock().await.finished = Some(Instant::now() - Duration::from_secs(31));
        }
        let expired = runtime.conversation(&store, &a, &seed, None).await.unwrap();
        assert!(
            !expired.complete && expired.continuation.is_some(),
            "cooldown cannot suppress verification indefinitely"
        );
        println!("REFRESH_BUDGET gmail={gmail} burst_api=200 burst_scans=0 initial_scans={discovery_scans} explicit_scans={explicit_scans}");
    }
}

#[tokio::test]
async fn slow_discovery_catches_membership_changes_and_keeps_snapshots_provider_free() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [43; 32]).await.unwrap();
    let user = store.user("fixture", "slow").await.unwrap();
    let a = account(&store, &user).await;
    let runtime = MailRuntime::default();
    let provider = Arc::new(Provider {
        account: a.id().into(),
        gmail: false,
        delay_ms: 25,
        fail: AtomicBool::new(false),
        scans: AtomicUsize::new(0),
        rows: vec![("INBOX".into(), raw("root", "", 26), 1)],
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
    let seed = runtime
        .page(&store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap()
        .rows[0]
        .id
        .clone();
    provider.scans.store(0, Ordering::SeqCst);
    let first = runtime.conversation(&store, &a, &seed, None).await.unwrap();
    let token = first.continuation.unwrap();
    let operation = runtime.conversation(&store, &a, &seed, Some(&token));
    let during = async {
        while provider.scans.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        // Semantic change while provider I/O is in flight, not display metadata.
        store
            .put_provider_document(&a, "thread:epoch:Empty", "2")
            .await
            .unwrap();
        for _ in 0..20 {
            let snapshot = runtime
                .conversation_read(&store, &a, &seed, None, true, false)
                .await
                .unwrap();
            assert_eq!(snapshot.rows.len(), 1);
            assert!(!snapshot.complete && snapshot.continuation.is_none());
        }
    };
    let (page, _) = tokio::join!(operation, during);
    let page = page.unwrap();
    assert!(
        page.continuation.is_some(),
        "changes during verification require final catch-up"
    );
    assert_eq!(provider.scans.load(Ordering::SeqCst), 4);
    let result = finish(&runtime, &store, &a, &seed, page).await;
    assert!(result.complete);
    assert_eq!(provider.scans.load(Ordering::SeqCst), 8);
    println!("SLOW_BUDGET snapshot_api=20 snapshot_scans=0 discovery_scans=4 catchup_scans=4 delay_ms=25");
}
