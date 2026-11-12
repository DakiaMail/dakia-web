//! Completed classification history must not monopolize the sync database.
use super::*;
use crate::models::CLASSIFICATION_CANDIDATE_QUERY;

#[tokio::test]
async fn classification_history_uses_active_index_and_releases_single_connection() {
    let (_dir, store, user, a, input) = fixture().await;
    let db = store.db(&a).await.unwrap();
    let mut template_message = message(&a, "template");
    template_message.classification = Some(Classification::default());
    let template = serde_json::to_string(&template_message).unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<2000) INSERT INTO messages(id,revision,document) SELECT printf('m%04d',i),0,json_set(?,'$.id',printf('m%04d',i),'$.received_at',printf('2026-10-29T00:%02d:%02dZ',i/60,i%60)) FROM n")
        .bind(template).execute(&db).await.unwrap();
    sqlx::query(
        "INSERT INTO message_identities(provider_id,message_id) VALUES('fixture-known','m0001')",
    )
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<2000) INSERT INTO model_jobs(id,state,document) SELECT printf('j%04d',i),CASE WHEN i%2=0 THEN 'succeeded' ELSE 'failed' END,json_object('message_id',printf('m%04d',i)) FROM n").execute(&db).await.unwrap();
    sqlx::query("INSERT INTO model_jobs(id,state,document) VALUES('active-pending','pending','{\"message_id\":\"m2000\"}'),('active-running','running','{\"message_id\":\"m1999\"}')").execute(&db).await.unwrap();
    sqlx::query("UPDATE messages SET category_override='people' WHERE id='m1998'")
        .execute(&db)
        .await
        .unwrap();
    let plan: Vec<String> = sqlx::query(&format!(
        "EXPLAIN QUERY PLAN {CLASSIFICATION_CANDIDATE_QUERY}"
    ))
    .fetch_all(&db)
    .await
    .unwrap()
    .into_iter()
    .map(|r| r.get::<String, _>("detail"))
    .collect();
    assert!(
        plan.iter().any(|s| s.contains("model_jobs_active_message")),
        "classification must not scan completed job history: {plan:?}"
    );
    assert!(
        plan.iter()
            .any(|s| s.contains("messages_pending_classification")),
        "classification must use the pending index: {plan:?}"
    );
    // Read mail remains eligible, as do manually classified messages needing type detection.
    sqlx::query("UPDATE messages SET document=json_set(document,'$.unread',0) WHERE id='m1997'")
        .execute(&db)
        .await
        .unwrap();
    // Real single-connection pool, concurrent classification and sync evidence.
    // The plan assertion above is deterministic; this is a generous liveness bound.
    let (candidate, known) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(
            store.classification_candidate(&a),
            store.known_provider_message(&a, "fixture-known")
        )
    })
    .await
    .expect("classification history starved sync evidence");
    assert_eq!(candidate.unwrap().unwrap().id, "m1998");
    assert_eq!(known.unwrap().unwrap().id, "m0001");
    // The same message/job IDs in another account must not influence this one.
    let b = store.connect(&user, &input).await.unwrap();
    let b = store.authorize(&user, &b.id).await.unwrap();
    assert!(store.classification_candidate(&b).await.unwrap().is_none());
    assert!(store
        .known_provider_message(&b, "fixture-known")
        .await
        .unwrap()
        .is_none());
    // Cached indexed access still revalidates ownership.
    sqlx::query("DELETE FROM accounts WHERE id=?")
        .bind(a.id())
        .execute(store.control())
        .await
        .unwrap();
    assert!(matches!(
        store.classification_candidate(&a).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.known_provider_message(&a, "fixture-known").await,
        Err(Error::NotFound)
    ));
}
