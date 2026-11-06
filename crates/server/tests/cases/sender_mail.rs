use super::*;
use sqlx::Row;

#[tokio::test]
async fn sender_catalogue_is_fast_local_owned_exact_paginated_and_read_neutral() {
    let (dir, app, alice, _) = setup().await;
    // An unreachable provider proves this route never waits for provider I/O.
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let one = app.store.connect(&alice, &input).await.unwrap();
    let two = app.store.connect(&alice, &input).await.unwrap();
    let a = app.store.authorize(&alice, &one.id).await.unwrap();
    let b = app.store.authorize(&alice, &two.id).await.unwrap();
    let base: Message = serde_json::from_value(serde_json::json!({"id":"m000","account_id":one.id,"thread_id":"fixture","provider_id":"fixture","folder_ids":["INBOX"],"sender":{"email":"list+tag@example.invalid","name":null},"to":[],"subject":"Synthetic list","preview":"Fixture","received_at":"2026-01-01T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    for n in 0..105 {
        let mut m = base.clone();
        m.id = format!("m{n:03}");
        app.store.upsert_message(&a, m).await.unwrap();
    }
    for (id, sender, folders, date) in [
        (
            "substring",
            "xlist+tag@example.invalid",
            vec!["INBOX"],
            "2026-01-01T00:00:00Z",
        ),
        (
            "case",
            "List+tag@example.invalid",
            vec!["INBOX"],
            "2026-01-01T00:00:00Z",
        ),
        (
            "trash",
            "list+tag@example.invalid",
            vec!["INBOX", "Deleted Items"],
            "2026-01-01T00:00:00Z",
        ),
        (
            "trash-role",
            "list+tag@example.invalid",
            vec!["\\Trash"],
            "2026-01-01T00:00:00Z",
        ),
        (
            "future",
            "list+tag@example.invalid",
            vec!["INBOX"],
            "2999-01-01T00:00:00Z",
        ),
        (
            "special-trash",
            "list+tag@example.invalid",
            vec!["Papierkorb"],
            "2026-01-01T00:00:00Z",
        ),
    ] {
        let mut m = base.clone();
        m.id = id.into();
        m.sender.email = sender.into();
        m.folder_ids = folders.into_iter().map(str::to_owned).collect();
        m.received_at = date.into();
        app.store.upsert_message(&a, m).await.unwrap();
    }
    let mut alien = base.clone();
    alien.account_id = two.id.clone();
    app.store.upsert_message(&b, alien).await.unwrap();
    app.store.put_provider_document(&a,"mailbox-settings",r#"{"trash_folder":"Deleted Items","received_cache_budget":1048576,"full_imap_exposure_confirmed":false,"sent_folder":null,"archive_folder":null}"#).await.unwrap();
    app.store.put_provider_document(&a,"probe",r#"{"capabilities":[],"folders":[{"name":"Papierkorb","selectable":true,"attributes":["\\Trash"]}],"full_search_coverage":false}"#).await.unwrap();
    let path = format!("/api/accounts/{}/messages/m000/sender-mail", one.id);
    for (token, status) in [
        ("", StatusCode::UNAUTHORIZED),
        ("bob-token", StatusCode::NOT_FOUND),
    ] {
        let r = router(app.clone())
            .oneshot(req("GET", &path, token, false, String::new()))
            .await
            .unwrap();
        assert_eq!(r.status(), status);
    }
    let get = |path: String| {
        let app = app.clone();
        async move {
            let r = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                router(app).oneshot(req("GET", &path, "alice-token", false, String::new())),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            assert_eq!(r.headers()["cache-control"], "no-store");
            serde_json::from_slice::<serde_json::Value>(
                &to_bytes(r.into_body(), 2_000_000).await.unwrap(),
            )
            .unwrap()
        }
    };
    let first = get(path.clone()).await;
    assert_eq!(first["count"], 105);
    assert_eq!(first["rows"].as_array().unwrap().len(), 100);
    assert_eq!(first["after"], "m099");
    let next = get(format!(
        "{path}?cutoff={}&after=m099",
        url::form_urlencoded::byte_serialize(first["cutoff"].as_str().unwrap().as_bytes())
            .collect::<String>()
    ))
    .await;
    assert_eq!(next["count"], 105);
    assert_eq!(next["rows"].as_array().unwrap().len(), 5);
    assert!(next["after"].is_null());
    assert!(app.store.message(&a, "m000").await.unwrap().unread);
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        dir.path()
            .join("accounts")
            .join(format!("{}.sqlite", one.id))
            .display()
    ))
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM operations")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    let plan: String=sqlx::query("EXPLAIN QUERY PLAN SELECT id FROM messages WHERE json_extract(document,'$.sender.email')='list+tag@example.invalid' AND id>'' ORDER BY id").fetch_one(&db).await.unwrap().get(3);
    assert!(plan.contains("messages_sender_id"), "{plan}");
    for query in ["cutoff=garbage", "cutoff=2999-01-01T00%3A00%3A00Z"] {
        let r = router(app.clone())
            .oneshot(req(
                "GET",
                &format!("{path}?{query}"),
                "alice-token",
                false,
                String::new(),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }
}
