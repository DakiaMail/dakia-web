use super::*;

#[tokio::test]
async fn inbox_seed_preserves_generations_and_discovers_new_legacy_rows_on_later_calls() {
    let (_dir, store, _user, a, _) = fixture().await;
    store
        .upsert_message(&a, message(&a, "known"))
        .await
        .unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query(
        "INSERT INTO folder_memberships(folder,message_id,generation) VALUES('INBOX','known',42)",
    )
    .execute(&db)
    .await
    .unwrap();
    store
        .upsert_message(&a, message(&a, "legacy"))
        .await
        .unwrap();
    let mut elsewhere = message(&a, "elsewhere");
    elsewhere.folder_ids = vec!["Archive".into()];
    store.upsert_message(&a, elsewhere.clone()).await.unwrap();
    store.seed_inbox_memberships(&a).await.unwrap();
    let rows: Vec<(String,i64)> = sqlx::query_as("SELECT message_id,generation FROM folder_memberships WHERE folder='INBOX' ORDER BY message_id").fetch_all(&db).await.unwrap();
    assert_eq!(rows, vec![("known".into(), 42), ("legacy".into(), -1)]);
    elsewhere.folder_ids = vec!["INBOX".into()];
    store.upsert_message(&a, elsewhere).await.unwrap();
    store
        .upsert_message(&a, message(&a, "later"))
        .await
        .unwrap();
    store.seed_inbox_memberships(&a).await.unwrap();
    let rows: Vec<(String,i64)> = sqlx::query_as("SELECT message_id,generation FROM folder_memberships WHERE folder='INBOX' ORDER BY message_id").fetch_all(&db).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("elsewhere".into(), -1),
            ("known".into(), 42),
            ("later".into(), -1),
            ("legacy".into(), -1)
        ]
    );
}

#[tokio::test]
#[ignore = "manual CPU timing for legacy seeding on an already cached inbox"]
async fn inbox_seed_cpu_timing() {
    let (_dir, store, _user, a, _) = fixture().await;
    for start in (0..5000).step_by(200) {
        store
            .ingest_messages(
                &a,
                (start..start + 200)
                    .map(|i| message(&a, &format!("seed-{i:05}")))
                    .collect(),
                &[],
            )
            .await
            .unwrap();
    }
    store.seed_inbox_memberships(&a).await.unwrap();
    let db = store.db(&a).await.unwrap();
    let started = std::time::Instant::now();
    for _ in 0..50 {
        sqlx::query("INSERT OR IGNORE INTO folder_memberships(folder,message_id,generation) SELECT 'INBOX',m.id,-1 FROM messages m WHERE EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE upper(f.value)='INBOX')").execute(&db).await.unwrap();
    }
    let old_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = std::time::Instant::now();
    for _ in 0..50 {
        store.seed_inbox_memberships(&a).await.unwrap();
    }
    println!(
        "SEED_CPU old_ms={old_ms:.3} indexed_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.
    );
}
