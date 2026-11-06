use super::*;
#[tokio::test]
async fn unsubscribe_is_owned_csrf_protected_and_claimed_once_without_network() {
    let (_dir, app, alice, bob) = setup().await;
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let one = app.store.connect(&alice, &input).await.unwrap();
    let two = app.store.connect(&alice, &input).await.unwrap();
    let foreign = app.store.connect(&bob, &input).await.unwrap();
    let a = app.store.authorize(&alice, &one.id).await.unwrap();
    let b = app.store.authorize(&alice, &two.id).await.unwrap();
    let message: Message = serde_json::from_value(serde_json::json!({"id":"synthetic","account_id":one.id,"thread_id":"fixture","provider_id":"fixture","folder_ids":["INBOX"],"sender":{"email":"newsletter@example.invalid","name":null},"to":[],"subject":"Synthetic list","preview":"Fixture","received_at":"2026-10-29T10:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    app.store.upsert_message(&a, message).await.unwrap();
    app.store
        .put_provider_document(
            &a,
            "unsubscribe-v1:synthetic",
            r#"{"kind":"web","url":"https://example.org/leave?private=fixture"}"#,
        )
        .await
        .unwrap();
    let path = format!("/api/accounts/{}/messages/synthetic/unsubscribe", one.id);
    for (method, token, csrf, expected) in [
        ("GET", "", false, StatusCode::UNAUTHORIZED),
        ("GET", "bob-token", false, StatusCode::NOT_FOUND),
        ("POST", "alice-token", false, StatusCode::FORBIDDEN),
        ("POST", "alice-token", true, StatusCode::BAD_REQUEST),
        ("GET", "alice-token", false, StatusCode::OK),
    ] {
        let response = router(app.clone())
            .oneshot(req(
                method,
                &path,
                token,
                csrf,
                r#"{"expected_url":"https://example.org/leave?private=fixture"}"#.into(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            assert_eq!(response.headers()["cache-control"], "no-store");
            let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["kind"],
                "web"
            );
        }
    }
    for account in [&two.id, &foreign.id] {
        let response = router(app.clone())
            .oneshot(req(
                "GET",
                &format!("/api/accounts/{account}/messages/synthetic/unsubscribe"),
                "alice-token",
                false,
                String::new(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    // Null support is returned accurately; no provider or network is needed.
    app.store
        .put_provider_document(&a, "unsubscribe-v1:synthetic", "null")
        .await
        .unwrap();
    let response = router(app.clone())
        .oneshot(req("GET", &path, "alice-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(
        to_bytes(response.into_body(), 8192).await.unwrap().as_ref(),
        b"null"
    );
    // A synthetic private target exercises production SSRF rejection, never sends.
    app.store
        .put_provider_document(
            &a,
            "unsubscribe-v1:synthetic",
            r#"{"kind":"one_click","url":"https://127.0.0.1/leave"}"#,
        )
        .await
        .unwrap();
    let response = router(app.clone())
        .oneshot(req(
            "POST",
            &path,
            "alice-token",
            true,
            r#"{"expected_url":"https://example.org/replaced"}"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(app
        .store
        .provider_document(&a, "unsubscribe-attempt:synthetic")
        .await
        .unwrap()
        .is_none());
    for expected in [StatusCode::BAD_REQUEST, StatusCode::CONFLICT] {
        let response = router(app.clone())
            .oneshot(req(
                "POST",
                &path,
                "alice-token",
                true,
                r#"{"expected_url":"https://127.0.0.1/leave"}"#.into(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let (first, second) = tokio::join!(
        app.store.claim_unsubscribe(&a, "unsubscribe-attempt:race"),
        app.store.claim_unsubscribe(&a, "unsubscribe-attempt:race")
    );
    assert_ne!(first.unwrap(), second.unwrap());
    assert!(app
        .store
        .claim_unsubscribe(&b, "unsubscribe-attempt:race")
        .await
        .unwrap());
    assert!(app.store.message(&a, "synthetic").await.unwrap().unread);
    // Email requests go into the same immutable durable submission pipeline,
    // never a client-authored draft or direct second SMTP path.
    let mailto = serde_json::json!({"kind":"mailto","to":"leave+synthetic@example.org","subject":"Leave+now","body":"Stop\r\nplease"});
    app.store
        .put_provider_document(&a, "unsubscribe-v1:synthetic", &mailto.to_string())
        .await
        .unwrap();
    // Reuse a fresh message ID so the earlier private-HTTP attempt is not replayed.
    let mut email_message = app.store.message(&a, "synthetic").await.unwrap();
    email_message.id = "email".into();
    app.store.upsert_message(&a, email_message).await.unwrap();
    app.store
        .put_provider_document(&a, "unsubscribe-v1:email", &mailto.to_string())
        .await
        .unwrap();
    let email_path = path.replace("synthetic", "email");
    let payload = serde_json::json!({"expected_action":mailto}).to_string();
    for (token, csrf, body, expected) in [
        ("bob-token", true, payload.clone(), StatusCode::NOT_FOUND),
        ("alice-token", false, payload.clone(), StatusCode::FORBIDDEN),
        ("alice-token", true, serde_json::json!({"expected_action":{"kind":"mailto","to":"other@example.org","subject":"Leave+now","body":"Stop\r\nplease"}}).to_string(), StatusCode::BAD_REQUEST),
        ("alice-token", true, serde_json::json!({"expected_action":{"kind":"mailto","to":"leave+synthetic@example.org","subject":"Leave+now","body":"Stop\r\nplease","bcc":"other@example.org"}}).to_string(), StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let response = router(app.clone()).oneshot(req("POST", &email_path, token, csrf, body)).await.unwrap();
        assert_eq!(response.status(), expected);
        assert!(app.store.submissions(&a).await.unwrap().is_empty());
    }
    for _ in 0..2 {
        let response = router(app.clone())
            .oneshot(req(
                "POST",
                &email_path,
                "alice-token",
                true,
                payload.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    let submissions = app.store.submissions(&a).await.unwrap();
    assert_eq!(submissions.len(), 1);
    let snapshot = app
        .store
        .submission_snapshot(&a, &submissions[0].id)
        .await
        .unwrap();
    assert_eq!(snapshot.to[0].email, "leave+synthetic@example.org");
    assert_eq!(snapshot.subject, "Leave+now");
    assert_eq!(snapshot.text, "Stop\r\nplease");
    assert!(snapshot.cc.is_empty() && snapshot.bcc.is_empty() && snapshot.attachments.is_empty());
    assert!(snapshot.in_reply_to.is_none() && snapshot.references.is_empty());
    app.store
        .put_provider_document(
            &a,
            "unsubscribe-v1:synthetic",
            r#"{"kind":"one_click","url":"https://127.0.0.1/leave"}"#,
        )
        .await
        .unwrap();
    // Reopening and a second POST recover a persisted successful request without
    // touching even this deliberately invalid network target. Legacy claims and
    // failed/uncertain results must never become successful or be replayed.
    for status in [
        "request_accepted",
        "request_rejected",
        "not_sent",
        "result_unknown",
        "attempted",
    ] {
        app.store
            .put_provider_document(&a, "unsubscribe-attempt:synthetic", status)
            .await
            .unwrap();
        let response = router(app.clone())
            .oneshot(req("GET", &path, "alice-token", false, String::new()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(value["attempt_status"], status);
        let response = router(app.clone())
            .oneshot(req(
                "POST",
                &path,
                "alice-token",
                true,
                r#"{"expected_url":"https://127.0.0.1/leave"}"#.into(),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if status == "request_accepted" {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::CONFLICT
            }
        );
        assert_eq!(
            app.store
                .provider_document(&a, "unsubscribe-attempt:synthetic")
                .await
                .unwrap()
                .as_deref(),
            Some(status)
        );
        assert!(app
            .store
            .provider_document(&b, "unsubscribe-attempt:synthetic")
            .await
            .unwrap()
            .is_none());
    }
}
