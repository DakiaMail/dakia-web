use super::*;

#[tokio::test]
async fn verified_components_expire_and_invalidate_on_local_changes() {
    let (_dir, store, a) = fixture().await;
    let mut m = message(&store, &a).await;
    let revision = store.thread_revision(&a).await.unwrap();
    store.verify_thread(&a, "component").await.unwrap();
    assert!(store
        .thread_verified(&a, "component", revision)
        .await
        .unwrap());
    m.unread = false;
    store.upsert_message(&a, m.clone()).await.unwrap();
    assert!(store
        .thread_verified(&a, "component", revision)
        .await
        .unwrap());
    m.folder_ids.clear();
    store.upsert_message(&a, m).await.unwrap();
    assert!(!store
        .thread_verified(&a, "component", revision)
        .await
        .unwrap());
    for (key, value) in [
        ("thread:epoch:INBOX", "1"),
        ("thread:epoch:INBOX", "2"),
        ("thread:folders", r#"{"names":["INBOX"],"at":0}"#),
        (
            "thread-headers:fixture",
            r#"{"ids":["new@example.invalid"],"gmail":null}"#,
        ),
    ] {
        store.verify_thread(&a, "component").await.unwrap();
        let revision = store.thread_revision(&a).await.unwrap();
        store.put_provider_document(&a, key, value).await.unwrap();
        assert!(
            !store
                .thread_verified(&a, "component", revision)
                .await
                .unwrap(),
            "{key}"
        );
    }
    store.verify_thread(&a, "component").await.unwrap();
    let revision = store.thread_revision(&a).await.unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE thread_verifications SET checked_at=0")
        .execute(&db)
        .await
        .unwrap();
    assert!(!store
        .thread_verified(&a, "component", revision)
        .await
        .unwrap());
    let stable = store.thread_revision(&a).await.unwrap();
    store
        .put_provider_document(
            &a,
            "thread:folders",
            r#"{"names":["INBOX"],"at":9999999999}"#,
        )
        .await
        .unwrap();
    assert_eq!(
        store.thread_revision(&a).await.unwrap(),
        stable,
        "a topology check is not a topology change"
    );
    store
        .verify_thread_revision(&a, "raced", stable - 1)
        .await
        .unwrap();
    assert!(!store.thread_verified(&a, "raced", stable).await.unwrap());
    // Persisted verification metadata has a fixed per-account bound.
    for i in 0..130 {
        store
            .verify_thread(&a, &format!("component-{i}"))
            .await
            .unwrap();
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM thread_verifications")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 128);
}
