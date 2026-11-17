//! Foreground reads must outlive neither backfill nor its per-message lock.
use super::*;

#[tokio::test]
async fn http_prefetch_stays_background_while_open_and_account_checks_bypass_it() {
    use axum::{body::Body, http::Request};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    let (_dir, app, user, a, provider) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let id = page.rows[0].id.clone();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(URL_SAFE_NO_PAD.encode(Sha256::digest(b"foreground-fixture")))
        .bind(&user)
        .bind("fixture")
        .bind(4102444800_i64)
        .execute(app.store.sqlite_control_for_test())
        .await
        .unwrap();
    let request = |path: String| {
        Request::builder()
            .uri(path)
            .header("cookie", "__Host-dakia=foreground-fixture")
            .body(Body::empty())
            .unwrap()
    };
    let router = crate::router(app.clone());
    let blocked = provider.background_fetch.lock().await;
    let prefetch = tokio::spawn(router.clone().oneshot(request(format!(
        "/api/accounts/{}/messages/{id}?prefetch=true",
        a.id()
    ))));
    tokio::time::timeout(
        Duration::from_secs(1),
        provider.background_started.notified(),
    )
    .await
    .unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(1),
        router
            .clone()
            .oneshot(request(format!("/api/accounts/{}/messages/{id}", a.id()))),
    )
    .await
    .expect("an HTTP open must bypass the stalled prefetch")
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(provider.foreground_fetch_calls.load(Ordering::SeqCst), 1);
    assert!(!prefetch.is_finished());
    let response = router
        .oneshot(request(format!(
            "/api/accounts/not-owned/messages/{id}?prefetch=true"
        )))
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
    drop(blocked);
    assert_eq!(prefetch.await.unwrap().unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn opening_the_same_body_bypasses_background_fetch_and_keeps_attachment_identity() {
    let (_dir, app, _, a, provider) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let id = page.rows[0].id.clone();
    let blocked = provider.background_fetch.lock().await;
    let background = {
        let (app, a, id) = (app.clone(), a.clone(), id.clone());
        tokio::spawn(async move { app.mail.content_background(&app.store, &a, &id).await })
    };
    tokio::time::timeout(
        Duration::from_secs(1),
        provider.background_started.notified(),
    )
    .await
    .unwrap();
    let foreground = tokio::time::timeout(
        Duration::from_secs(1),
        app.mail.content(&app.store, &a, &id),
    )
    .await
    .expect("opening must not wait for a background body or its message lock")
    .unwrap();
    assert!(!background.is_finished());
    assert!(!foreground.attachments.is_empty());
    assert_eq!(provider.foreground_fetch_calls.load(Ordering::SeqCst), 1);
    drop(blocked);
    let background = background.await.unwrap().unwrap();
    assert_eq!(foreground.attachments, background.attachments);
    assert_eq!(
        app.mail
            .content(&app.store, &a, &id)
            .await
            .unwrap()
            .attachments,
        foreground.attachments
    );
}

#[tokio::test]
async fn attachment_download_bypasses_background_body_repair() {
    let (_dir, app, _, a, provider) = controlled().await;
    let page = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    let id = page.rows[0].id.clone();
    let content = app.mail.content(&app.store, &a, &id).await.unwrap();
    app.store.evict_received(&a).await.unwrap();
    let blocked = provider.background_fetch.lock().await;
    let background = {
        let (app, a, id) = (app.clone(), a.clone(), id.clone());
        tokio::spawn(async move { app.mail.content_background(&app.store, &a, &id).await })
    };
    tokio::time::timeout(
        Duration::from_secs(1),
        provider.background_started.notified(),
    )
    .await
    .unwrap();
    let (attachment, bytes) = tokio::time::timeout(
        Duration::from_secs(1),
        app.mail
            .received_part(&app.store, &a, &content.attachments[0].id),
    )
    .await
    .expect("attachment downloads must not wait for background repair")
    .unwrap();
    assert_eq!(attachment, content.attachments[0]);
    assert!(!bytes.is_empty());
    assert!(!background.is_finished());
    drop(blocked);
    assert_eq!(
        background.await.unwrap().unwrap().attachments,
        content.attachments
    );
}
