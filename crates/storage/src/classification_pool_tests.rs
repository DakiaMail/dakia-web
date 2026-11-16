//! Completed classification history must not monopolize the sync database.
use super::*;

#[tokio::test]
async fn expired_model_claims_recover_without_reclaiming_active_or_committed_jobs() {
    let (_dir, store, _, a, _) = fixture().await;
    let mut m = message(&a, "expiry");
    m.classification = Some(Classification::default());
    store.upsert_message(&a, m).await.unwrap();
    store
        .queue_model(&a, "expiry", JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let job = store.claim_model(&a).await.unwrap().unwrap();
    assert!(store.claim_model(&a).await.unwrap().is_none());
    assert_eq!(
        store.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Running
    );
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE model_jobs SET document=json_set(document,'$.deadline','2000-01-01T00:00:00Z') WHERE id=?").bind(&job.id).execute(&db).await.unwrap();
    assert!(store.claim_model(&a).await.unwrap().is_none());
    assert_eq!(
        store.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Failed
    );
    assert!(store.classification_candidate(&a).await.unwrap().is_some());
    store
        .queue_model(&a, "expiry", JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let mut committed = store.claim_model(&a).await.unwrap().unwrap();
    committed.status = JobStatus::Succeeded;
    store
        .finish_model(&a, committed.clone(), Some("people"), None)
        .await
        .unwrap();
    let revision = store.message(&a, "expiry").await.unwrap().revision;
    sqlx::query("UPDATE model_jobs SET document=json_set(document,'$.deadline','2000-01-01T00:00:00Z') WHERE id=?").bind(&committed.id).execute(&db).await.unwrap();
    assert!(store.claim_model(&a).await.unwrap().is_none());
    store
        .finish_model(&a, committed.clone(), Some("newsletters"), None)
        .await
        .unwrap();
    assert_eq!(
        store.model_job(&a, &committed.id).await.unwrap().status,
        JobStatus::Succeeded
    );
    assert_eq!(
        store.message(&a, "expiry").await.unwrap().revision,
        revision
    );
}

#[tokio::test]
async fn failed_result_persistence_can_defer_both_model_kinds() {
    let (_dir, store, _, a, _) = fixture().await;
    let db = store.db(&a).await.unwrap();
    for kind in [JobKind::Classify, JobKind::Translate] {
        store
            .upsert_message(&a, message(&a, "defer-result"))
            .await
            .unwrap();
        store
            .queue_model(&a, "defer-result", kind, "fixture", None)
            .await
            .unwrap();
        let job = store.claim_model(&a).await.unwrap().unwrap();
        assert!(store
            .defer_model_failure(&a, &job, ModelFailureReason::StorageUnavailable)
            .await
            .unwrap());
        assert_eq!(
            store.model_job(&a, &job.id).await.unwrap().status,
            JobStatus::Pending
        );
        assert!(store.claim_model(&a).await.unwrap().is_none());
        sqlx::query("UPDATE model_jobs SET retry_at=0 WHERE id=?")
            .bind(&job.id)
            .execute(&db)
            .await
            .unwrap();
        let mut retried = store.claim_model(&a).await.unwrap().unwrap();
        retried.status = JobStatus::Cancelled;
        store.finish_model(&a, retried, None, None).await.unwrap();
    }
}
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
        .execute(store.sqlite_control_for_test())
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
