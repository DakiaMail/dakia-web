use super::*;
use axum::body::HttpBody;
use std::{pin::Pin, time::Duration};
async fn frame(body: &mut Body) -> String {
    let frame = tokio::time::timeout(
        Duration::from_secs(3),
        std::future::poll_fn(|cx| Pin::new(&mut *body).poll_frame(cx)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap()
}
#[tokio::test]
async fn event_stream_tails_history_replays_valid_cursors_and_delivers_new_changes() {
    let (_dir, app, user, _) = setup().await;
    let input = serde_json::from_value(serde_json::json!({"email":"test@example.invalid","display_name":"Test","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let account = app.store.connect(&user, &input).await.unwrap();
    let scope = app.store.authorize(&user, &account.id).await.unwrap();
    let message = |i| {
        serde_json::from_value(serde_json::json!({"id":format!("m{i}"),"account_id":account.id,"thread_id":"thread","provider_id":format!("p{i}"),"folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},"to":[],"subject":"fixture","preview":"","received_at":"2026-10-28T10:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap()
    };
    for i in 0..20 {
        app.store.upsert_message(&scope, message(i)).await.unwrap();
    }
    let latest = app.store.event_cursor(&scope).await.unwrap();
    assert_eq!(latest, 20);
    let routes = router(app.clone());
    for after in [0, latest + 100] {
        let response = routes
            .clone()
            .oneshot(req(
                "GET",
                &format!("/api/accounts/{}/events?after={after}", account.id),
                "alice-token",
                false,
                String::new(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        let ready = frame(&mut body).await;
        assert!(ready.contains("event: ready"));
        assert!(ready.contains(&format!("\"cursor\":{latest}")));
        assert!(ready.contains("\"reset\":true"));
        // No historical message payload follows the ready frame.
        assert!(tokio::time::timeout(
            Duration::from_millis(30),
            std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
        )
        .await
        .is_err());
    }
    let response = routes
        .clone()
        .oneshot(req(
            "GET",
            &format!("/api/accounts/{}/events?after=19", account.id),
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    let mut body = response.into_body();
    assert!(frame(&mut body).await.contains("\"reset\":false"));
    assert!(frame(&mut body).await.contains("\"cursor\":20"));
    app.store.upsert_message(&scope, message(21)).await.unwrap();
    let changed = frame(&mut body).await;
    assert!(changed.contains("\"cursor\":21"));
    assert!(changed.contains("\"target_id\":\"m21\""));
}
