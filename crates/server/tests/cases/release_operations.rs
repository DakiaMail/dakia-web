use super::*;

async fn account(app: &App, user: &str) -> dakia_storage::AuthorizedAccount {
    let input = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"synthetic-only"})).unwrap();
    let a = app.store.connect(user, &input).await.unwrap();
    app.store.authorize(user, &a.id).await.unwrap()
}
async fn message(app: &App, a: &dakia_storage::AuthorizedAccount) -> Message {
    let m = serde_json::from_value(serde_json::json!({"id":"message","account_id":a.id(),"thread_id":"thread","provider_id":"synthetic","folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"synthetic","preview":"","received_at":"2026-10-28T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    app.store.upsert_message(a, m).await.unwrap()
}

macro_rules! action_contract {
    ($name:ident, $change:expr, $status:expr) => {
        #[tokio::test]
        async fn $name() {
            let (_dir, app, user, _) = setup().await;
            let a = account(&app, &user).await;
            let m = message(&app, &a).await;
            let op = Operation {
                id: "client-operation".into(),
                account_id: a.id().into(),
                target_id: m.id.clone(),
                expected_revision: m.revision,
                change: $change,
                status: OperationStatus::Applied,
                revision: 999,
            };
            let path = format!("/api/accounts/{}/operations", a.id());
            for (token, csrf, status) in [
                ("", true, StatusCode::UNAUTHORIZED),
                ("bob-token", true, StatusCode::NOT_FOUND),
                ("alice-token", false, StatusCode::FORBIDDEN),
            ] {
                let response = router(app.clone())
                    .oneshot(req(
                        "POST",
                        &path,
                        token,
                        csrf,
                        serde_json::to_string(&op).unwrap(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(response.status(), status);
                assert_eq!(
                    app.store.message(&a, &m.id).await.unwrap(),
                    m,
                    "rejected request changed mail"
                );
            }
            let mut accepted = None;
            for _ in 0..2 {
                let response = router(app.clone())
                    .oneshot(req(
                        "POST",
                        &path,
                        "alice-token",
                        true,
                        serde_json::to_string(&op).unwrap(),
                    ))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(response.headers()["cache-control"], "no-store");
                let result: Operation =
                    serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap())
                        .unwrap();
                assert_eq!(
                    result.status, $status,
                    "client may not claim provider work already applied"
                );
                if let Some(previous) = accepted {
                    assert_eq!(result, previous);
                }
                accepted = Some(result);
            }
            let mut conflicting = op.clone();
            conflicting.change = Change::Move {
                folder_id: "Different".into(),
            };
            let response = router(app.clone())
                .oneshot(req(
                    "POST",
                    &path,
                    "alice-token",
                    true,
                    serde_json::to_string(&conflicting).unwrap(),
                ))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::CONFLICT,
                "same id cannot mean a second operation"
            );
            let mut stale = op;
            stale.id = "stale-operation".into();
            let response = router(app.clone())
                .oneshot(req(
                    "POST",
                    &path,
                    "alice-token",
                    true,
                    serde_json::to_string(&stale).unwrap(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
    };
}
action_contract!(
    archive_http_contract,
    Change::Archive,
    OperationStatus::Pending
);
action_contract!(trash_http_contract, Change::Trash, OperationStatus::Pending);
action_contract!(spam_http_contract, Change::Spam, OperationStatus::Pending);
action_contract!(
    move_http_contract,
    Change::Move {
        folder_id: "Projects".into()
    },
    OperationStatus::Pending
);
action_contract!(
    mark_read_http_contract,
    Change::Read { value: true },
    OperationStatus::Pending
);
action_contract!(
    mark_unread_http_contract,
    Change::Read { value: false },
    OperationStatus::Pending
);
action_contract!(
    star_http_contract,
    Change::Star { value: true },
    OperationStatus::Pending
);
action_contract!(
    unstar_http_contract,
    Change::Star { value: false },
    OperationStatus::Pending
);
action_contract!(
    category_http_contract,
    Change::Category {
        value: "people".into()
    },
    OperationStatus::Applied
);
action_contract!(
    snooze_http_contract,
    Change::Snooze {
        until: "2099-01-01T00:00:00Z".into()
    },
    OperationStatus::Applied
);

#[tokio::test]
async fn attachment_download_preserves_binary_bytes_and_owner_only_access() {
    let (_dir, app, user, _) = setup().await;
    let a = account(&app, &user).await;
    let bytes: Vec<u8> = (0..=255).cycle().take(131_073).collect();
    let attachment = app
        .store
        .put_attachment(&a, "report.pdf".into(), "application/pdf".into(), &bytes)
        .await
        .unwrap();
    let path = format!("/api/accounts/{}/attachments/{}", a.id(), attachment.id);
    for (token, status) in [
        ("", StatusCode::UNAUTHORIZED),
        ("bob-token", StatusCode::NOT_FOUND),
        ("alice-token", StatusCode::OK),
    ] {
        let response = router(app.clone())
            .oneshot(req("GET", &path, token, false, String::new()))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "no-store");
        if status == StatusCode::OK {
            assert_eq!(
                response.headers()["content-type"],
                "application/octet-stream"
            );
            assert!(response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("sandbox"));
            assert_eq!(
                to_bytes(response.into_body(), 200_000)
                    .await
                    .unwrap()
                    .as_ref(),
                bytes
            );
        }
    }
}

#[tokio::test]
async fn attachment_download_preserves_filename_and_extension() {
    let (_dir, app, user, _) = setup().await;
    let a = account(&app, &user).await;
    let attachment = app
        .store
        .put_attachment(
            &a,
            "invoice-2026.pdf".into(),
            "application/pdf".into(),
            b"%PDF-fixture",
        )
        .await
        .unwrap();
    let response = router(app)
        .oneshot(req(
            "GET",
            &format!("/api/accounts/{}/attachments/{}", a.id(), attachment.id),
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    assert!(
        disposition.contains("invoice-2026.pdf"),
        "download must preserve the usable filename, got {disposition}"
    );
}

#[tokio::test]
async fn missing_attachment_is_not_successful_empty_download() {
    let (_dir, app, user, _) = setup().await;
    let a = account(&app, &user).await;
    let response = router(app)
        .oneshot(req(
            "GET",
            &format!("/api/accounts/{}/attachments/missing", a.id()),
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unicode_attachment_filename_is_encoded_and_cannot_be_a_path_or_header() {
    let (_dir, app, user, _) = setup().await;
    let a = account(&app, &user).await;
    let attachment = app
        .store
        .put_attachment(
            &a,
            "../../résumé \"2026\".pdf".into(),
            "application/pdf".into(),
            b"fixture",
        )
        .await
        .unwrap();
    let response = router(app)
        .oneshot(req(
            "GET",
            &format!("/api/accounts/{}/attachments/{}", a.id(), attachment.id),
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-disposition"], "attachment; filename=\"r_sum_ _2026_.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9%20%222026%22.pdf");
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert!(response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("sandbox"));
}

#[tokio::test]
async fn sync_activity_polling_does_not_change_mail_or_start_provider_work() {
    let (_dir, app, user, _) = setup().await;
    let a = account(&app, &user).await;
    let m = message(&app, &a).await;
    for _ in 0..25 {
        let response = router(app.clone())
            .oneshot(req(
                "GET",
                &format!("/api/accounts/{}/sync-active", a.id()),
                "alice-token",
                false,
                String::new(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
            b"false"
        );
    }
    assert_eq!(app.store.message(&a, &m.id).await.unwrap(), m);
    assert!(app
        .store
        .provider_document(&a, "sync:folders")
        .await
        .unwrap()
        .is_none());
    assert!(app
        .store
        .provider_document(&a, "sync:head:INBOX")
        .await
        .unwrap()
        .is_none());
}
