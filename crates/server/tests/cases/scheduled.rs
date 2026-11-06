use super::*;

#[tokio::test]
async fn scheduled_view_filters_history_uses_snapshot_and_preserves_audit_and_isolation() {
    let (dir, app, alice, _) = setup().await;
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"test@example.invalid","display_name":"Test","provider":"imap","imap_host":"imap.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.invalid","smtp_port":465,"smtp_tls":"implicit","username":"test","password":"fixture-only"})).unwrap();
    let account = app.store.connect(&alice, &input).await.unwrap();
    let other = app.store.connect(&alice, &input).await.unwrap();
    let scope = app.store.authorize(&alice, &account.id).await.unwrap();
    let dbpath = dir
        .path()
        .join("accounts")
        .join(format!("{}.sqlite", account.id));
    let db = sqlx::SqlitePool::connect(&format!("sqlite:{}", dbpath.display()))
        .await
        .unwrap();
    let draft: Draft = serde_json::from_value(serde_json::json!({"id":"draft","account_id":account.id,"revision":1,"to":[{"email":"recipient@example.invalid","name":"Recipient"}],"cc":[],"bcc":[],"subject":"Immutable scheduled subject","text":"private body","html":"","in_reply_to":null,"references":[],"attachments":[],"saved_at":"2026-10-29T10:00:00Z"})).unwrap();
    let mut expected = Vec::new();
    let scenarios = [
        ("scheduled", "pending", true, true),
        ("claimed", "pending", true, true),
        ("submitting", "pending", true, true),
        ("failed", "pending", true, true),
        ("delivery_unknown", "pending", true, true),
        ("accepted", "failed", true, true),
        ("accepted", "filed", true, false),
        ("accepted", "not_required", true, false),
        ("cancelled", "pending", true, false),
        ("scheduled", "pending", false, false),
        ("failed", "pending", false, false),
        ("accepted", "failed", false, false),
    ];
    for (index, (state, filing, scheduled, visible)) in scenarios.into_iter().enumerate() {
        let id = format!("s{index}");
        let due = format!("2027-11-{:02}T10:00:00Z", index + 1);
        let document = serde_json::json!({"id":id,"account_id":account.id,"operation_id":id,"draft_id":"draft","draft_revision":1,"state":state,"due_at":due,"timezone":"Europe/Tallinn","filing_status":filing,"revision":1});
        let request =
            serde_json::json!({"due_at": if scheduled { Some(due.clone()) } else { None }});
        sqlx::query("INSERT INTO submissions(id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES(?,?,?,'draft',1,?,?,?,?,?)")
            .bind(&id).bind(&id).bind(request.to_string()).bind(serde_json::to_string(&draft).unwrap()).bind(document.to_string()).bind(state).bind(&due).bind(filing).execute(&db).await.unwrap();
        if visible {
            expected.push(id);
        }
    }
    let path = format!("/api/accounts/{}/scheduled", account.id);
    for (token, expected_status) in [
        ("", StatusCode::UNAUTHORIZED),
        ("bob-token", StatusCode::NOT_FOUND),
    ] {
        let response = router(app.clone())
            .oneshot(req("GET", &path, token, false, String::new()))
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status);
    }
    let response = router(app.clone())
        .oneshot(req("GET", &path, "alice-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 100_000).await.unwrap();
    let rows: Vec<ScheduledMessage> = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r.submission.id.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(rows.iter().all(|r| r.subject == draft.subject
        && r.to == draft.to
        && r.submission.timezone == "Europe/Tallinn"));
    assert!(!String::from_utf8_lossy(&body).contains("private body"));
    assert_eq!(
        app.store.submissions(&scope).await.unwrap().len(),
        scenarios.len()
    );
    let other_scope = app.store.authorize(&alice, &other.id).await.unwrap();
    assert!(app
        .store
        .scheduled_messages(&other_scope)
        .await
        .unwrap()
        .is_empty());
    db.close().await;
}
