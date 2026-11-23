use super::*;
use dakia_storage::{CleanupRequest, CleanupTarget};

#[tokio::test]
async fn cleanup_worker_drains_bounded_turns_for_48_and_200_targets() {
    for count in [48, 200] {
        let (_dir, app, _, a, provider) = controlled().await;
        let mut rows = app
            .mail
            .snapshot_page(&app.store, &a, "INBOX", None)
            .await
            .unwrap()
            .rows;
        let raw = app
            .store
            .provider_document(&a, &format!("reference:{}", rows[0].id))
            .await
            .unwrap()
            .unwrap();
        let reference: p::MessageRef = serde_json::from_str(&raw).unwrap();
        for n in 2..count {
            let mut row = rows[0].clone();
            row.id = format!("cleanup-{n}");
            row.provider_id = format!("fixture-{n}");
            let row = app.store.upsert_message(&a, row).await.unwrap();
            let mut reference = reference.clone();
            reference.uid = n as u32 + 1;
            app.store
                .put_provider_document(
                    &a,
                    &format!("reference:{}", row.id),
                    &serde_json::to_string(&reference).unwrap(),
                )
                .await
                .unwrap();
            rows.push(row);
        }
        let request = CleanupRequest {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: rows[0].id.clone(),
            search_complete: true,
            targets: rows
                .iter()
                .map(|m| CleanupTarget {
                    id: m.id.clone(),
                    expected_revision: m.revision,
                    expected_location: None,
                })
                .collect(),
        };
        app.store
            .enqueue_sender_cleanup(&a, request.clone())
            .await
            .unwrap();
        assert_eq!(provider.mutations.lock().await.len(), 0);
        crate::mutations::drain(&app, &a).await.unwrap();
        let first_turn = provider.mutations.lock().await.len();
        // A turn also yields after two seconds, so a busy test host may
        // legitimately process fewer than the maximum 20 writes.
        assert!(
            (1..=20).contains(&first_turn),
            "one bounded worker turn must make progress without exceeding 20 writes: {first_turn}"
        );
        while app.store.sender_cleanup_jobs(&a).await.unwrap()[0].pending > 0 {
            crate::mutations::drain(&app, &a).await.unwrap();
        }
        let calls = provider.mutations.lock().await.len();
        assert_eq!(calls, count);
        let job = app.store.enqueue_sender_cleanup(&a, request).await.unwrap();
        assert_eq!((job.applied, job.failed, job.pending), (count, 0, 0));
        crate::mutations::drain(&app, &a).await.unwrap();
        assert_eq!(provider.mutations.lock().await.len(), calls);
        println!("cleanup fixture: {count} targets, {calls} provider mutations, zero provider calls at acceptance, no replay");
    }
}

#[tokio::test]
async fn cleanup_acceptance_and_status_do_not_wait_for_slow_provider_and_keep_partial_failure() {
    let (dir, app, _, a, provider) = controlled().await;
    let rows = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows;
    let request = CleanupRequest {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: rows[0].id.clone(),
        search_complete: true,
        targets: rows
            .iter()
            .map(|m| CleanupTarget {
                id: m.id.clone(),
                expected_revision: m.revision,
                expected_location: Some(dakia_storage::CleanupLocation {
                    provider_id: m.provider_id.clone(),
                    folder_ids: m.folder_ids.clone(),
                    sender_email: m.sender.email.clone(),
                }),
            })
            .collect(),
    };
    // Reproduce the real reader race: search captured the source before its
    // automatic mark-read was queued. Cleanup must follow, not reject it.
    app.store
        .operation(
            &a,
            Operation {
                id: "reader-mark-read".into(),
                account_id: a.id().into(),
                target_id: rows[0].id.clone(),
                expected_revision: rows[0].revision,
                change: Change::Read { value: true },
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    let accepted = app
        .store
        .enqueue_sender_cleanup(&a, request.clone())
        .await
        .unwrap();
    assert_eq!(accepted.pending, 2);
    assert!(provider.mutations.lock().await.is_empty());
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(provider.mutations.lock().await.len(), 1);
    // The provider blocks inside mutate, not in storage or the acceptance path.
    let gate = provider.page.lock().await;
    let worker = {
        let app = app.clone();
        let a = a.clone();
        tokio::spawn(async move { crate::mutations::tick(&app, &a).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while provider.mutations.lock().await.len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let job = tokio::time::timeout(
        Duration::from_millis(500),
        app.store.enqueue_sender_cleanup(&a, request.clone()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(job.pending, 2);
    assert_eq!(job.applied, 0);
    let jobs = tokio::time::timeout(
        Duration::from_millis(500),
        app.store.sender_cleanup_jobs(&a),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(jobs[0].pending, 2);
    assert!(!worker.is_finished());
    drop(gate);
    worker.await.unwrap().unwrap();
    *provider.failure.lock().await = Some(p::ErrorKind::Retryable);
    crate::mutations::tick(&app, &a).await.unwrap();
    let job = app.store.sender_cleanup_jobs(&a).await.unwrap().remove(0);
    assert_eq!((job.applied, job.failed, job.pending), (1, 0, 1));
    let attempted = provider.mutations.lock().await.len();
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(
        provider.mutations.lock().await.len(),
        attempted,
        "durable backoff prevents busy retries"
    );
    let db = database(&dir, &a).await;
    sqlx::query("UPDATE outbox SET lease_until='2000-01-01T00:00:00.000Z' WHERE state='pending'")
        .execute(&db)
        .await
        .unwrap();
    *provider.failure.lock().await = Some(p::ErrorKind::Uncertain);
    crate::mutations::tick(&app, &a).await.unwrap();
    let job = app.store.sender_cleanup_jobs(&a).await.unwrap().remove(0);
    assert_eq!((job.applied, job.failed, job.pending), (1, 1, 0));
    let attempted = provider.mutations.lock().await.len();
    app.store.enqueue_sender_cleanup(&a, request).await.unwrap();
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(
        provider.mutations.lock().await.len(),
        attempted,
        "uncertain moves never replay"
    );
    assert_eq!(job.failed_targets.len(), 1);
    db.close().await;
}

#[tokio::test]
async fn ordinary_bulk_actions_drain_together_without_replay() {
    let (_dir, app, _, a, provider) = controlled().await;
    let rows = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        app.store
            .operation(
                &a,
                Operation {
                    id: uuid::Uuid::new_v4().to_string(),
                    account_id: a.id().into(),
                    target_id: row.id.clone(),
                    expected_revision: row.revision,
                    change: Change::Archive,
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
    }
    assert!(provider.mutations.lock().await.is_empty());
    crate::mutations::drain(&app, &a).await.unwrap();
    assert_eq!(provider.mutations.lock().await.len(), 2);
    for row in &rows {
        assert!(!app
            .store
            .message(&a, &row.id)
            .await
            .unwrap()
            .folder_ids
            .contains(&"INBOX".into()));
    }
    crate::mutations::drain(&app, &a).await.unwrap();
    assert_eq!(provider.mutations.lock().await.len(), 2);
}

#[tokio::test]
async fn blocked_sender_review_uses_local_move_eligible_messages() {
    use axum::{body::Body, http::Request};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    let (_dir, app, user, a, provider) = controlled().await;
    let rows = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 2);
    app.store
        .operation(
            &a,
            Operation {
                id: "blocked-sender-unresolved-move".into(),
                account_id: a.id().into(),
                target_id: rows[0].id.clone(),
                expected_revision: rows[0].revision,
                change: Change::Archive,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(URL_SAFE_NO_PAD.encode(Sha256::digest(b"sender-review-fixture")))
        .bind(&user)
        .bind("fixture")
        .bind(4102444800_i64)
        .execute(app.store.sqlite_control_for_test())
        .await
        .unwrap();

    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("sender", &rows[1].sender.email.to_uppercase())
        .append_pair("cutoff", &chrono::Utc::now().to_rfc3339())
        .finish();
    let scans = provider.base.scans.load(Ordering::SeqCst);
    let router = crate::router(app.clone());
    let catalogue = format!("/api/accounts/{}/sender-mail?{query}", a.id());
    let unsubscribe_catalogue = format!(
        "/api/accounts/{}/messages/{}/sender-mail",
        a.id(),
        rows[1].id
    );
    let mut reviewed = rows[1].clone();
    for url in [catalogue, unsubscribe_catalogue] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(url)
                    .header("cookie", "__Host-dakia=sender-review-fixture")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(body["count"], 1);
        reviewed = serde_json::from_value(body["rows"][0].clone()).unwrap();
        assert_eq!(reviewed.id, rows[1].id);
    }
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), scans);

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/accounts/{}/sender-cleanups", a.id()))
                .header("cookie", "__Host-dakia=sender-review-fixture")
                .header("x-csrf-token", "fixture")
                .header("origin", app.config.origin.as_str())
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&CleanupRequest {
                        id: uuid::Uuid::new_v4().to_string(),
                        source_id: reviewed.id.clone(),
                        search_complete: false,
                        targets: vec![CleanupTarget {
                            id: reviewed.id,
                            expected_revision: reviewed.revision,
                            expected_location: Some(dakia_storage::CleanupLocation {
                                provider_id: reviewed.provider_id,
                                folder_ids: reviewed.folder_ids,
                                sender_email: reviewed.sender.email,
                            }),
                        }],
                    })
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        app.store.sender_cleanup_jobs(&a).await.unwrap()[0].pending,
        1
    );
    assert!(provider.mutations.lock().await.is_empty());
}
