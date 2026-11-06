use super::*;

#[tokio::test]
async fn cleanup_routes_are_owned_csrf_protected_atomic_local_and_idempotent() {
    let (_dir, app, alice, _) = setup().await;
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let one = app.store.connect(&alice, &input).await.unwrap();
    let two = app.store.connect(&alice, &input).await.unwrap();
    let a = app.store.authorize(&alice, &one.id).await.unwrap();
    let m: Message = serde_json::from_value(serde_json::json!({"id":"source","account_id":one.id,"thread_id":"fixture","provider_id":"fixture","folder_ids":["INBOX"],"sender":{"email":"list@example.invalid","name":null},"to":[],"subject":"Synthetic","preview":"Fixture","received_at":"2026-01-01T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    let m = app.store.upsert_message(&a, m).await.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let body = serde_json::json!({"id":id,"source_id":"source","search_complete":false,"targets":[{"id":"source","expected_revision":m.revision}]}).to_string();
    let path = format!("/api/accounts/{}/sender-cleanups", one.id);
    for (method, token, csrf, expected) in [
        ("GET", "", false, StatusCode::UNAUTHORIZED),
        ("GET", "bob-token", false, StatusCode::NOT_FOUND),
        ("POST", "alice-token", false, StatusCode::FORBIDDEN),
        ("POST", "bob-token", true, StatusCode::NOT_FOUND),
    ] {
        let r = router(app.clone())
            .oneshot(req(method, &path, token, csrf, body.clone()))
            .await
            .unwrap();
        assert_eq!(r.status(), expected);
    }
    assert!(app.store.claim_mutation(&a).await.unwrap().is_none());
    for _ in 0..2 {
        let r = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            router(app.clone()).oneshot(req("POST", &path, "alice-token", true, body.clone())),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["cache-control"], "no-store");
        let job: serde_json::Value =
            serde_json::from_slice(&to_bytes(r.into_body(), 100000).await.unwrap()).unwrap();
        assert_eq!(job["pending"], 1);
        assert_eq!(job["applied"], 0);
    }
    let wrong = format!("/api/accounts/{}/sender-cleanups", two.id);
    let r = router(app.clone())
        .oneshot(req("POST", &wrong, "alice-token", true, body))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let op = app.store.claim_mutation(&a).await.unwrap().unwrap();
    assert_eq!(op.change, Change::Trash);
    assert!(app.store.claim_mutation(&a).await.unwrap().is_none());
    app.store.recover_mutations(&a).await.unwrap();
    let r = router(app.clone())
        .oneshot(req("GET", &path, "alice-token", false, String::new()))
        .await
        .unwrap();
    let jobs: serde_json::Value =
        serde_json::from_slice(&to_bytes(r.into_body(), 100000).await.unwrap()).unwrap();
    assert_eq!(jobs[0]["failed"], 1);
    assert_eq!(jobs[0]["failed_targets"][0], "source");
    let dismiss = format!("{path}/{id}/dismiss");
    let r = router(app.clone())
        .oneshot(req("POST", &dismiss, "bob-token", true, String::new()))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = router(app.clone())
        .oneshot(req("POST", &dismiss, "alice-token", true, String::new()))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let r = router(app)
        .oneshot(req("GET", &path, "alice-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(&to_bytes(r.into_body(), 100000).await.unwrap()[..], b"[]");
}

#[tokio::test]
async fn cached_sender_targets_remain_usable_after_reader_mark_read() {
    let (_dir, app, alice, _) = setup().await;
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let account = app.store.connect(&alice, &input).await.unwrap();
    let a = app.store.authorize(&alice, &account.id).await.unwrap();
    let message: Message = serde_json::from_value(serde_json::json!({"id":"source","account_id":account.id,"thread_id":"fixture","provider_id":"fixture","folder_ids":["INBOX"],"sender":{"email":"list@example.invalid","name":null},"to":[],"subject":"Synthetic","preview":"Fixture","received_at":"2026-01-01T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    app.store.upsert_message(&a, message).await.unwrap();
    let base = format!("/api/accounts/{}", account.id);
    let response = router(app.clone())
        .oneshot(req(
            "GET",
            &format!("{base}/messages/source/sender-mail"),
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 100000).await.unwrap()).unwrap();
    let row = &page["rows"][0];
    let operation = serde_json::json!({"id":"reader-read","account_id":account.id,"target_id":"source","expected_revision":row["revision"],"change":{"kind":"read","value":true},"status":"pending","revision":0});
    let response = router(app.clone())
        .oneshot(req(
            "POST",
            &format!("{base}/operations"),
            "alice-token",
            true,
            operation.to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let request = serde_json::json!({"id":uuid::Uuid::new_v4().to_string(),"source_id":"source","search_complete":false,"targets":[{"id":"source","expected_revision":row["revision"],"expected_location":{"provider_id":row["provider_id"],"folder_ids":row["folder_ids"],"sender_email":row["sender"]["email"]}}]});
    for _ in 0..2 {
        let response = router(app.clone())
            .oneshot(req(
                "POST",
                &format!("{base}/sender-cleanups"),
                "alice-token",
                true,
                request.to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let mut read = app.store.claim_mutation(&a).await.unwrap().unwrap();
    assert!(matches!(read.change, Change::Read { .. }));
    read.status = OperationStatus::Applied;
    app.store
        .finish_mutation(&a, read, Some(vec!["INBOX".into()]))
        .await
        .unwrap();
    let trash = app.store.claim_mutation(&a).await.unwrap().unwrap();
    assert_eq!(trash.change, Change::Trash);
    assert!(app.store.claim_mutation(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn cleanup_reports_unverified_archive_without_enqueuing_trash() {
    let (_dir, app, alice, _) = setup().await;
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let account = app.store.connect(&alice, &input).await.unwrap();
    let a = app.store.authorize(&alice, &account.id).await.unwrap();
    let message: Message = serde_json::from_value(serde_json::json!({"id":"source","account_id":account.id,"thread_id":"fixture","provider_id":"fixture","folder_ids":["INBOX","All Mail"],"sender":{"email":"list@example.invalid","name":null},"to":[],"subject":"Synthetic","preview":"Fixture","received_at":"2026-01-01T00:00:00Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    let message = app.store.upsert_message(&a, message).await.unwrap();
    app.store
        .operation(
            &a,
            Operation {
                id: "prior-archive".into(),
                account_id: account.id.clone(),
                target_id: message.id.clone(),
                expected_revision: message.revision,
                change: Change::Archive,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    let mut archive = app.store.claim_mutation(&a).await.unwrap().unwrap();
    archive.status = OperationStatus::Failed;
    app.store.finish_mutation(&a, archive, None).await.unwrap();
    let current = app.store.message(&a, "source").await.unwrap();
    let body = serde_json::json!({"id":uuid::Uuid::new_v4().to_string(),"source_id":"source","search_complete":false,"targets":[{"id":"source","expected_revision":current.revision,"expected_location":{"provider_id":current.provider_id,"folder_ids":current.folder_ids,"sender_email":current.sender.email}}]}).to_string();
    let response = router(app.clone())
        .oneshot(req(
            "POST",
            &format!("/api/accounts/{}/sender-cleanups", account.id),
            "alice-token",
            true,
            body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 100000).await.unwrap()).unwrap();
    assert_eq!(error["code"], "cleanup_move_unverified");
    assert!(app.store.sender_cleanup_jobs(&a).await.unwrap().is_empty());
    assert!(app.store.claim_mutation(&a).await.unwrap().is_none());
    assert_eq!(
        app.store.message(&a, "source").await.unwrap().folder_ids,
        current.folder_ids
    );
}
