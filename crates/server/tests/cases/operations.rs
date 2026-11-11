use super::*;
#[tokio::test]
async fn operation_status_is_read_only_scoped_and_reports_durable_outcome() {
    let (_dir, app, alice, _) = setup().await;
    let input = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"synthetic-only"})).unwrap();
    let account = app.store.connect(&alice, &input).await.unwrap();
    let a = app.store.authorize(&alice, &account.id).await.unwrap();
    let m: Message = serde_json::from_value(serde_json::json!({"id":"message","account_id":a.id(),"thread_id":"thread","provider_id":"synthetic","folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"synthetic","preview":"","received_at":"2026-10-28T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    let m = app.store.upsert_message(&a, m).await.unwrap();
    let op = app
        .store
        .operation(
            &a,
            Operation {
                id: "archive".into(),
                account_id: a.id().into(),
                target_id: m.id,
                expected_revision: m.revision,
                change: Change::Archive,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    let path = format!("/api/accounts/{}/operations/{}", a.id(), op.id);
    for status in [OperationStatus::Pending, OperationStatus::Applied] {
        if status == OperationStatus::Applied {
            let mut claimed = app.store.claim_mutation(&a).await.unwrap().unwrap();
            claimed.status = status.clone();
            app.store
                .finish_mutation(&a, claimed, Some(vec!["Archive".into()]))
                .await
                .unwrap();
        }
        for (token, expected) in [
            ("", StatusCode::UNAUTHORIZED),
            ("bob-token", StatusCode::NOT_FOUND),
            ("alice-token", StatusCode::OK),
        ] {
            let r = router(app.clone())
                .oneshot(req("GET", &path, token, false, String::new()))
                .await
                .unwrap();
            assert_eq!(r.status(), expected);
            if expected == StatusCode::OK {
                assert_eq!(r.headers()["cache-control"], "no-store");
                let found: Operation =
                    serde_json::from_slice(&to_bytes(r.into_body(), 4096).await.unwrap()).unwrap();
                assert_eq!(found.status, status);
                assert_eq!(found.account_id, a.id());
            }
        }
    }
}

#[tokio::test]
async fn snooze_metadata_is_owned_and_never_public_or_cross_account() {
    let (_dir, app, alice, _) = setup().await;
    let input = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"synthetic-only"})).unwrap();
    let account = app.store.connect(&alice, &input).await.unwrap();
    let path = format!("/api/accounts/{}/snoozes", account.id);
    for (token, expected) in [("", StatusCode::UNAUTHORIZED), ("bob-token", StatusCode::NOT_FOUND), ("alice-token", StatusCode::OK)] {
        let response = router(app.clone()).oneshot(req("GET", &path, token, false, String::new())).await.unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body: serde_json::Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
            assert_eq!(body, serde_json::json!([]));
        }
    }
}
