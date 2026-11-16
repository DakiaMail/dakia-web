use super::*;
#[tokio::test]
async fn removal_is_confirmed_local_isolated_and_revokes_stale_capabilities() {
    let (d, app, alice, _) = setup().await;
    let input = ConnectAccount {
        email: "remove@example.invalid".into(),
        display_name: "Remove".into(),
        provider: ProviderKind::Gmail,
        imap_host: "must-not-connect.invalid".into(),
        imap_port: 993,
        imap_tls: TlsMode::Implicit,
        smtp_host: "must-not-connect.invalid".into(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        username: "synthetic".into(),
        password: "fixture-only".into(),
        gmail_oauth: None,
        microsoft_oauth: None,
    };
    let a = app.store.connect(&alice, &input).await.unwrap();
    let other = app.store.connect(&alice, &input).await.unwrap();
    let scope = app.store.authorize(&alice, &a.id).await.unwrap();
    let other_scope = app.store.authorize(&alice, &other.id).await.unwrap();
    let blob = app
        .store
        .put_attachment(&scope, "test.txt".into(), "text/plain".into(), b"synthetic")
        .await
        .unwrap();
    app.store
        .put_attachment(
            &other_scope,
            "kept.txt".into(),
            "text/plain".into(),
            b"keep",
        )
        .await
        .unwrap();
    let path = format!("/api/accounts/{}", a.id);
    let confirmed = serde_json::json!({"confirm_email":input.email}).to_string();
    for (token, csrf, body, expected) in [
        (
            "alice-token",
            false,
            confirmed.clone(),
            StatusCode::FORBIDDEN,
        ),
        ("bob-token", true, confirmed.clone(), StatusCode::NOT_FOUND),
        (
            "alice-token",
            true,
            "{\"confirm_email\":\"wrong\"}".into(),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = router(app.clone())
            .oneshot(req("DELETE", &path, token, csrf, body))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let dbpath = d.path().join("accounts").join(format!("{}.sqlite", a.id));
    let db = sqlx::SqlitePool::connect(&format!("sqlite:{}", dbpath.display()))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO outbox(kind,target_id,state) VALUES('mutation','synthetic','running')",
    )
    .execute(&db)
    .await
    .unwrap();
    let response = router(app.clone())
        .oneshot(req("DELETE", &path, "alice-token", true, confirmed.clone()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(app.store.credential(&scope).await.is_ok());
    sqlx::query("DELETE FROM outbox")
        .execute(&db)
        .await
        .unwrap();
    for (state, filing) in [
        ("scheduled", "pending"),
        ("claimed", "pending"),
        ("submitting", "pending"),
        ("delivery_unknown", "pending"),
        ("accepted", "pending"),
        ("accepted", "failed"),
    ] {
        sqlx::query("INSERT INTO submissions(id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES('busy','busy','{}','draft',1,'{}','{}',?,'2026-01-01',?)")
            .bind(state).bind(filing).execute(&db).await.unwrap();
        let response = router(app.clone())
            .oneshot(req("DELETE", &path, "alice-token", true, confirmed.clone()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT, "{state}/{filing}");
        sqlx::query("DELETE FROM submissions")
            .execute(&db)
            .await
            .unwrap();
    }
    // A disconnected account must be removable without replaying queued changes.
    app.store
        .account_state(&scope, AccountState::Reconnect)
        .await
        .unwrap();
    for (i, change) in [
        Change::Star { value: true },
        Change::Archive,
        Change::Star { value: true },
        Change::Star { value: true },
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("queued-{i}");
        let message: Message = serde_json::from_value(serde_json::json!({"id":id,"account_id":scope.id(),"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"Synthetic","preview":"","received_at":"2026-10-28T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
        let message = app.store.upsert_message(&scope, message).await.unwrap();
        app.store
            .operation(
                &scope,
                Operation {
                    id: id.clone(),
                    account_id: scope.id().into(),
                    target_id: id,
                    expected_revision: message.revision,
                    change,
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM outbox WHERE kind='mutation' AND state='pending'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        4
    );
    db.close().await;
    let response = router(app.clone())
        .oneshot(req("DELETE", &path, "alice-token", true, confirmed.clone()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(app.store.authorize(&alice, &a.id).await.is_err());
    assert!(app.store.credential(&scope).await.is_err());
    assert!(app.store.attachment(&scope, &blob.id).await.is_err());
    assert!(!dbpath.exists());
    assert!(!d.path().join("durable").join(&a.id).exists());
    assert_eq!(app.store.accounts(&alice).await.unwrap()[0].id, other.id);
    assert!(app.store.credential(&other_scope).await.is_ok());
    let response = router(app.clone())
        .oneshot(req("DELETE", &path, "alice-token", true, confirmed))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let orphan = d.path().join("cache").join(&a.id);
    tokio::fs::create_dir_all(&orphan).await.unwrap();
    tokio::fs::write(orphan.join("synthetic"), b"orphan")
        .await
        .unwrap();
    let reopened = Store::open(d.path(), [7; 32]).await.unwrap();
    assert!(!orphan.exists());
    assert_eq!(reopened.accounts(&alice).await.unwrap()[0].id, other.id);
}
