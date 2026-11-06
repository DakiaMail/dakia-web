use super::*;

#[tokio::test]
async fn incremental_observations_checkpoint_and_notifications_never_finish_or_prune_snapshot() {
    let (dir, store, _, a, _) = fixture().await;
    let old = store.upsert_message(&a, message(&a, "old")).await.unwrap();
    store.seed_inbox_memberships(&a).await.unwrap();
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE folder_scans SET continuation='historical',coverage_limited=1,initial_boundary=0 WHERE folder='INBOX'").execute(&db).await.unwrap();
    let before = store.scan_state(&a, "INBOX").await.unwrap();
    let arrival = store
        .upsert_message(&a, message(&a, "arrival"))
        .await
        .unwrap();
    for _ in 0..2 {
        store
            .record_incremental(&a, "INBOX", std::slice::from_ref(&arrival), "checkpoint")
            .await
            .unwrap();
        assert_eq!(store.scan_state(&a, "INBOX").await.unwrap(), before);
        assert!(store
            .message(&a, &old.id)
            .await
            .unwrap()
            .folder_ids
            .contains(&"INBOX".into()));
    }
    let status = store.inbox_coverage(&a).await.unwrap();
    assert_eq!(status.state, CoverageState::Pending);
    assert!(status.inbox.unwrap().last_scanned_at.is_none());
    let dedup: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM notification_dedup WHERE message_id='arrival'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(dedup, 1);
    let membership: i64 =
        sqlx::query_scalar("SELECT generation FROM folder_memberships WHERE message_id='arrival'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(membership, generation);
    let mut foreign = arrival.clone();
    foreign.account_id = "other".into();
    assert!(store
        .record_incremental(&a, "INBOX", &[arrival, foreign], "must-not-advance")
        .await
        .is_err());
    assert_eq!(
        store
            .provider_document(&a, "sync:head:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("checkpoint")
    );
    // Empty incremental results are not authoritative absence either.
    store
        .record_incremental(&a, "INBOX", &[], "empty-head")
        .await
        .unwrap();
    assert_eq!(store.scan_state(&a, "INBOX").await.unwrap(), before);
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    assert_eq!(
        reopened
            .provider_document(&a, "sync:head:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("empty-head")
    );
}

#[tokio::test]
async fn coordinated_ingestion_accepts_200_and_rejects_count_and_document_overruns_atomically() {
    let (_dir, store, _, a, _) = fixture().await;
    let rows = store
        .ingest_messages(
            &a,
            (0..200).map(|i| message(&a, &format!("m{i}"))).collect(),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 200);
    assert!(store
        .ingest_messages(
            &a,
            (0..201)
                .map(|i| message(&a, &format!("overflow{i}")))
                .collect(),
            &[]
        )
        .await
        .is_err());
    assert!(matches!(
        store.message(&a, "overflow0").await,
        Err(Error::NotFound)
    ));
    assert!(store
        .ingest_messages(
            &a,
            vec![message(&a, "byte-overflow")],
            &[("large".into(), "x".repeat(16 * 1024 * 1024))]
        )
        .await
        .is_err());
    assert!(matches!(
        store.message(&a, "byte-overflow").await,
        Err(Error::NotFound)
    ));
    assert!(store
        .provider_document(&a, "large")
        .await
        .unwrap()
        .is_none());
}
