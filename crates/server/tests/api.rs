use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use dakia_domain::*;
use dakia_server::{router, App, Config};
use dakia_storage::Store;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;
#[path = "cases/events.rs"]
mod events;
#[path = "cases/operations.rs"]
mod operations;
#[path = "cases/release_operations.rs"]
mod release_operations;
#[path = "cases/removal.rs"]
mod removal;
#[path = "cases/scheduled.rs"]
mod scheduled;
#[path = "cases/sender_cleanup.rs"]
mod sender_cleanup;
#[path = "cases/sender_mail.rs"]
mod sender_mail;
#[path = "cases/unsubscribe.rs"]
mod unsubscribe;
async fn setup() -> (tempfile::TempDir, App, String, String) {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path(), [7; 32]).await.unwrap();
    let a = store
        .user("https://id.example.invalid", "alice")
        .await
        .unwrap();
    let b = store
        .user("https://id.example.invalid", "bob")
        .await
        .unwrap();
    for (user, token) in [(&a, "alice-token"), (&b, "bob-token")] {
        let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()));
        sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
            .bind(hash)
            .bind(user)
            .bind("csrf-value")
            .bind(4102444800_i64)
            .execute(store.control())
            .await
            .unwrap();
    }
    let config = Config {
        origin: "https://dakia.example.invalid".into(),
        issuer: "https://id.example.invalid".into(),
        client_id: "test".into(),
        client_secret: "fixture-only".into(),
        web_root: d.path().to_string_lossy().into(),
    };
    (
        d,
        App {
            store,
            config: Arc::new(config),
            mail: Default::default(),
        },
        a,
        b,
    )
}
fn req(method: &str, url: &str, token: &str, csrf: bool, body: String) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(url)
        .header("cookie", format!("__Host-dakia={token}"))
        .header("content-type", "application/json");
    if csrf {
        r = r
            .header("origin", "https://dakia.example.invalid")
            .header("x-csrf-token", "csrf-value")
    }
    r.body(Body::from(body)).unwrap()
}
#[tokio::test]
async fn anonymous_and_csrf_denied() {
    let (_d, app, _, _) = setup().await;
    let r = router(app.clone())
        .oneshot(req("GET", "/api/accounts", "", false, String::new()))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let r = router(app.clone())
        .oneshot(req(
            "POST",
            "/api/accounts",
            "alice-token",
            false,
            "{}".into(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = router(app)
        .oneshot(req(
            "GET",
            "/api/session",
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
}
#[tokio::test]
async fn sync_activity_requires_login_and_does_not_create_accounts() {
    let (_d, app, user, _) = setup().await;
    let path = "/api/accounts/nonexistent/sync-active";
    for (token, status) in [
        ("", StatusCode::UNAUTHORIZED),
        ("alice-token", StatusCode::NOT_FOUND),
    ] {
        let r = router(app.clone())
            .oneshot(req("GET", path, token, false, String::new()))
            .await
            .unwrap();
        assert_eq!(r.status(), status);
    }
    assert!(app.store.accounts(&user).await.unwrap().is_empty());
}
#[tokio::test]
async fn discovery_is_authenticated_csrf_protected_and_never_creates_an_account() {
    let (_d, app, user, _) = setup().await;
    for (token, csrf, expected) in [
        ("", false, StatusCode::UNAUTHORIZED),
        ("alice-token", false, StatusCode::FORBIDDEN),
        ("alice-token", true, StatusCode::OK),
    ] {
        let r = router(app.clone())
            .oneshot(req(
                "POST",
                "/api/accounts/discover",
                token,
                csrf,
                r#"{"email":"synthetic@gmail.com"}"#.into(),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), expected);
        if expected == StatusCode::OK {
            assert_eq!(r.headers()["cache-control"], "no-store");
            let bytes = to_bytes(r.into_body(), 16_384).await.unwrap();
            let found: DiscoveredSettings = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(found.provider, ProviderKind::Gmail);
            assert_eq!(found.imap_host, "imap.gmail.com");
            assert_eq!(found.smtp_tls, TlsMode::Implicit);
            assert_eq!(found.username, "synthetic@gmail.com");
        }
    }
    let r = router(app.clone())
        .oneshot(req(
            "POST",
            "/api/accounts/discover",
            "alice-token",
            true,
            r#"{"email":"x@127.0.0.1"}"#.into(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(app.store.accounts(&user).await.unwrap().is_empty());
}
#[tokio::test]
async fn isolation_including_events_and_drafts() {
    let (_d, app, alice, _) = setup().await;
    let input = ConnectAccount {
        email: "test@example.invalid".into(),
        display_name: "Test".into(),
        provider: ProviderKind::Imap,
        imap_host: "imap.example.invalid".into(),
        imap_port: 993,
        imap_tls: TlsMode::Implicit,
        smtp_host: "smtp.example.invalid".into(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        username: "test".into(),
        password: "fixture-only".into(),
    };
    let a = app.store.connect(&alice, &input).await.unwrap();
    let scope = app.store.authorize(&alice, &a.id).await.unwrap();
    let r = router(app.clone())
        .oneshot(req(
            "GET",
            &format!("/api/accounts/{}/sync-active", a.id),
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(
        to_bytes(r.into_body(), 128).await.unwrap().as_ref(),
        b"false"
    );
    let attachment = app
        .store
        .put_attachment(&scope, "test.txt".into(), "text/plain".into(), b"synthetic")
        .await
        .unwrap();
    for path in [
        format!("/api/accounts/{}/drafts/test", a.id),
        format!("/api/accounts/{}/events", a.id),
        format!("/api/accounts/{}/submissions/test", a.id),
        format!("/api/accounts/{}/operations/test", a.id),
        format!("/api/accounts/{}/drafts", a.id),
        format!("/api/accounts/{}/submissions", a.id),
        format!("/api/accounts/{}/recipients?q=test", a.id),
        format!("/api/accounts/{}/folders", a.id),
        format!("/api/accounts/{}/sync-active", a.id),
        format!("/api/accounts/{}/settings", a.id),
        format!(
            "/api/accounts/{}/messages/synthetic/conversation?continuation=foreign",
            a.id
        ),
        format!("/api/search?accounts={}", a.id),
        format!("/api/accounts/{}/attachments/{}", a.id, attachment.id),
    ] {
        let r = router(app.clone())
            .oneshot(req("GET", &path, "bob-token", false, String::new()))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let download = router(app.clone())
        .oneshot(req(
            "GET",
            &format!("/api/accounts/{}/attachments/{}", a.id, attachment.id),
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        download.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(
        &to_bytes(download.into_body(), 100).await.unwrap()[..],
        b"synthetic"
    );
    for (path, body) in [
        (
            format!("/api/accounts/{}/submissions", a.id),
            serde_json::json!({"operation_id":"should-not-queue","draft_id":"absent","expected_revision":1,"due_at":null,"timezone":"UTC"}),
        ),
        (
            format!("/api/accounts/{}/operations", a.id),
            serde_json::json!({"id":"should-not-queue","account_id":a.id,"target_id":"absent","expected_revision":1,"change":{"kind":"read","value":true},"status":"pending","revision":0}),
        ),
    ] {
        let response = router(app.clone())
            .oneshot(req("POST", &path, "alice-token", true, body.to_string()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
    assert!(app.store.submissions(&scope).await.unwrap().is_empty());
    let no_accounts = router(app.clone())
        .oneshot(req("GET", "/api/search", "bob-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(no_accounts.status(), StatusCode::OK);
    let page: SearchPage =
        serde_json::from_slice(&to_bytes(no_accounts.into_body(), 4096).await.unwrap()).unwrap();
    assert!(page.rows.is_empty() && page.coverage.is_empty());
    let r = router(app)
        .oneshot(req(
            "GET",
            "/api/accounts",
            "bob-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    let bytes = to_bytes(r.into_body(), 4096).await.unwrap();
    assert_eq!(&bytes[..], b"[]");
}
#[tokio::test]
async fn image_proxy_requires_csrf_owned_account_and_existing_message() {
    let (_dir, app, alice, _) = setup().await;
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = app.store.connect(&alice, &input).await.unwrap();
    let path = format!("/api/accounts/{}/messages/missing/image", account.id);
    for (token, csrf, expected) in [
        ("alice-token", false, StatusCode::FORBIDDEN),
        ("bob-token", true, StatusCode::NOT_FOUND),
        ("alice-token", true, StatusCode::NOT_FOUND),
    ] {
        let response = router(app.clone())
            .oneshot(req(
                "POST",
                &path,
                token,
                csrf,
                serde_json::json!({"url":"https://httpbin.org/image/png"}).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}
#[tokio::test]
async fn logout_revokes_session() {
    let (_d, app, _, _) = setup().await;
    let r = router(app.clone())
        .oneshot(req(
            "POST",
            "/api/logout",
            "alice-token",
            true,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let r = router(app)
        .oneshot(req(
            "GET",
            "/api/session",
            "alice-token",
            false,
            String::new(),
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
#[test]
fn configuration_requires_https() {
    let c = Config {
        origin: "http://localhost".into(),
        issuer: "https://id.example.invalid".into(),
        client_id: "test".into(),
        client_secret: "test".into(),
        web_root: "dist".into(),
    };
    assert!(c.validate().is_err());
}

#[tokio::test]
async fn reconnect_is_owned_in_place_and_preserves_data() {
    let (_d, app, alice, _) = setup().await;
    let mut input = ConnectAccount {
        email: "test@example.invalid".into(),
        display_name: "Test".into(),
        provider: ProviderKind::Imap,
        imap_host: "imap.example.invalid".into(),
        imap_port: 993,
        imap_tls: TlsMode::Implicit,
        smtp_host: "smtp.example.invalid".into(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        username: "test".into(),
        password: "old-fixture-only".into(),
    };
    let account = app.store.connect(&alice, &input).await.unwrap();
    let scope = app.store.authorize(&alice, &account.id).await.unwrap();
    let blob = app
        .store
        .put_attachment(&scope, "kept.txt".into(), "text/plain".into(), b"keep")
        .await
        .unwrap();
    input.password = "replacement-fixture-only".into();
    let path = format!("/api/accounts/{}", account.id);
    for (token, csrf, status) in [
        ("bob-token", true, StatusCode::NOT_FOUND),
        ("alice-token", false, StatusCode::FORBIDDEN),
        ("alice-token", true, StatusCode::OK),
    ] {
        let response = router(app.clone())
            .oneshot(req(
                "PUT",
                &path,
                token,
                csrf,
                serde_json::to_string(&input).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("replacement-fixture-only"));
    }
    assert_eq!(
        app.store.credential(&scope).await.unwrap().password,
        input.password
    );
    assert_eq!(app.store.accounts(&alice).await.unwrap().len(), 1);
    assert_eq!(
        app.store.attachment(&scope, &blob.id).await.unwrap().1,
        b"keep"
    );
    input.email = "different@example.invalid".into();
    let response = router(app.clone())
        .oneshot(req(
            "PUT",
            &path,
            "alice-token",
            true,
            serde_json::to_string(&input).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        app.store.accounts(&alice).await.unwrap()[0].email,
        account.email
    );
}
