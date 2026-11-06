use super::*;
async fn blocked_fixture() -> (
    tempfile::TempDir,
    Store,
    AuthorizedAccount,
    Message,
    String,
    ContentReferences,
) {
    let (dir, store, _, a, _) = fixture().await;
    let mut m = message(&a, "blocked");
    m.provider_id = "gmail:99".into();
    let m = store.upsert_message(&a, m).await.unwrap();
    let reference =
        serde_json::json!({"account_id":a.id(),"folder":"INBOX","uidvalidity":1,"uid":1})
            .to_string();
    store
        .put_provider_document(&a, &format!("reference:{}", m.id), &reference)
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    let state = store.content_references(&a, &m).await.unwrap();
    assert!(store
        .block_content_reference(&a, &m, &reference, &state.token)
        .await
        .unwrap());
    assert!(store
        .mark_reference_gap(&a, &m, &state.token)
        .await
        .unwrap());
    (dir, store, a, m, reference, state)
}
#[tokio::test]
async fn reference_gap_preserves_daily_cooldown_across_foreground_reads_and_restart() {
    let (dir, store, a, m, reference, state) = blocked_fixture().await;
    assert!(store.claim_reference_recovery(&a).await.unwrap().is_some());
    let db = store.db(&a).await.unwrap();
    let due: i64 = sqlx::query_scalar("SELECT retry_at FROM search_documents WHERE message_id=?")
        .bind(&m.id)
        .fetch_one(&db)
        .await
        .unwrap();
    for _ in 0..24 {
        assert!(store
            .mark_reference_gap(&a, &m, &state.token)
            .await
            .unwrap());
        assert!(store.next_search_body(&a, false).await.unwrap().is_none());
    }
    let next: i64 = sqlx::query_scalar("SELECT retry_at FROM search_documents WHERE message_id=?")
        .bind(&m.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(due, next);
    sqlx::query("UPDATE provider_documents SET document='0' WHERE key='content:recovery_due'")
        .execute(&db)
        .await
        .unwrap();
    assert!(
        store.claim_reference_recovery(&a).await.unwrap().is_none(),
        "per-message cooldown outlives account budget"
    );
    let user = store.user("fixture", "owner").await.unwrap();
    let account = a.id().to_owned();
    drop(db);
    drop(store);
    let store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = store.authorize(&user, &account).await.unwrap();
    let restored = store.content_references(&a, &m).await.unwrap();
    assert!(restored.blocked.contains(&reference));
    assert!(store.next_search_body(&a, false).await.unwrap().is_none());
}
#[tokio::test]
async fn reference_gap_observation_fences_and_transaction_failure_preserve_freshness() {
    let (_dir, store, a, m, reference, state) = blocked_fixture().await;
    let db = store.db(&a).await.unwrap();
    let mut foreign = m.clone();
    foreign.account_id = "foreign-account".into();
    assert!(store
        .promote_content_reference(&a, &foreign, &reference, &state.token)
        .await
        .is_err());
    let docs = vec![
        (format!("reference:{}", m.id), reference.clone()),
        (format!("__content_observation:{}", m.id), "1".into()),
    ];
    store
        .ingest_messages(&a, vec![m.clone()], &docs)
        .await
        .unwrap();
    assert!(
        store.next_search_body(&a, false).await.unwrap().is_none(),
        "an observation started before the missing UID cannot rearm it"
    );
    let fresh = vec![
        (format!("reference:{}", m.id), reference.clone()),
        (
            format!("__content_observation:{}", m.id),
            Utc::now().timestamp_micros().to_string(),
        ),
    ];
    sqlx::raw_sql("CREATE TRIGGER fail_rearm BEFORE UPDATE OF body_state ON search_documents WHEN NEW.body_state='pending' BEGIN SELECT RAISE(ABORT,'fixture'); END;").execute(&db).await.unwrap();
    assert!(store
        .ingest_messages(&a, vec![m.clone()], &fresh)
        .await
        .is_err());
    assert!(store
        .content_references(&a, &m)
        .await
        .unwrap()
        .blocked
        .contains(&reference));
    sqlx::query("DROP TRIGGER fail_rearm")
        .execute(&db)
        .await
        .unwrap();
    store
        .ingest_messages(&a, vec![m.clone()], &fresh)
        .await
        .unwrap();
    assert!(store.next_search_body(&a, false).await.unwrap().is_some());
    assert!(
        !store
            .block_content_reference(&a, &m, &reference, &state.token)
            .await
            .unwrap(),
        "old failure cannot override a fresh observation"
    );
}
