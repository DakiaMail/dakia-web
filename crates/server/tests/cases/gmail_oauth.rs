use super::*;
use dakia_server::gmail_oauth::GmailConfig;

fn configured(mut app: App) -> App {
    app.mail = app.mail.with_gmail(Some(GmailConfig {
        client_id: "development.apps.googleusercontent.com".into(),
        client_secret: "fixture-only-secret".into(),
    }));
    app
}
async fn start(app: &App, token: &str) -> axum::response::Response {
    router(app.clone())
        .oneshot(req(
            "POST",
            "/api/accounts/gmail",
            token,
            true,
            r#"{"email":"alice@example.test","display_name":"Personal","account_id":null}"#.into(),
        ))
        .await
        .unwrap()
}
async fn authorization(response: axum::response::Response) -> url::Url {
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    url::Url::parse(body["url"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn gmail_requires_configuration_login_and_csrf_and_uses_pkce_offline_mail_scope() {
    let (_d, app, _, _) = setup().await;
    assert_eq!(
        start(&app, "alice-token").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let app = configured(app);
    assert_eq!(start(&app, "").await.status(), StatusCode::UNAUTHORIZED);
    let response = router(app.clone())
        .oneshot(req(
            "POST",
            "/api/accounts/gmail",
            "alice-token",
            false,
            "{}".into(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let url = authorization(start(&app, "alice-token").await).await;
    assert_eq!(url.host_str(), Some("accounts.google.com"));
    let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(q["access_type"], "offline");
    assert_eq!(q["prompt"], "consent");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["scope"], "openid email https://mail.google.com/");
    assert_eq!(
        q["redirect_uri"],
        "https://dakia.example.invalid/api/accounts/gmail/callback"
    );
    assert!(!q.contains_key("client_secret"));
    let verifier: String = sqlx::query_scalar("SELECT verifier FROM gmail_oauth_flows")
        .fetch_one(app.store.control())
        .await
        .unwrap();
    assert_eq!(
        q["code_challenge"],
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    );
}

#[tokio::test]
async fn gmail_callback_is_session_bound_single_use_expiring_and_cancel_safe() {
    let (_d, app, alice, _) = setup().await;
    let app = configured(app);
    let url = authorization(start(&app, "alice-token").await).await;
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let path = format!("/api/accounts/gmail/callback?state={state}&error=access_denied");
    let response = router(app.clone())
        .oneshot(req("GET", &path, "bob-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM gmail_oauth_flows")
        .fetch_one(app.store.control())
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "another session must not consume the owner's flow"
    );
    let response = router(app.clone())
        .oneshot(req("GET", &path, "alice-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/?gmail_error=cancelled");
    let response = router(app.clone())
        .oneshot(req("GET", &path, "alice-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(response.headers()["location"], "/?gmail_error=expired");
    assert!(app.store.accounts(&alice).await.unwrap().is_empty());
    let url = authorization(start(&app, "alice-token").await).await;
    let state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned();
    let path = format!("/api/accounts/gmail/callback?state={state}&error=access_denied");
    sqlx::query("UPDATE gmail_oauth_flows SET expires_at=0")
        .execute(app.store.control())
        .await
        .unwrap();
    let response = router(app.clone())
        .oneshot(req("GET", &path, "alice-token", false, String::new()))
        .await
        .unwrap();
    assert_eq!(response.headers()["location"], "/?gmail_error=expired");
}

#[tokio::test]
async fn gmail_reconnect_rejects_other_users_and_wrong_accounts() {
    let (_d, app, alice, _) = setup().await;
    let app = configured(app);
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({
        "email":"alice@example.test", "display_name":"Personal", "provider":"gmail",
        "imap_host":"imap.gmail.com", "imap_port":993,"imap_tls":"implicit",
        "smtp_host":"smtp.gmail.com","smtp_port":465,"smtp_tls":"implicit", "username":"alice@example.test","password":"synthetic"
    })).unwrap();
    let account = app.store.connect(&alice, &input).await.unwrap();
    for (token, email, status) in [
        ("bob-token", "alice@example.test", StatusCode::NOT_FOUND),
        ("alice-token", "wrong@example.test", StatusCode::BAD_REQUEST),
    ] {
        let response = router(app.clone()).oneshot(req("POST", "/api/accounts/gmail", token, true,
            serde_json::json!({"email":email,"display_name":"Personal","account_id":account.id}).to_string())).await.unwrap();
        assert_eq!(response.status(), status);
    }
}

#[tokio::test]
async fn gmail_refresh_credentials_are_encrypted_and_not_accepted_through_password_api() {
    let (_d, app, alice, _) = setup().await;
    let mut input: ConnectAccount = serde_json::from_value(serde_json::json!({
        "email":"alice@example.test", "display_name":"Personal", "provider":"gmail",
        "imap_host":"imap.gmail.com", "imap_port":993,"imap_tls":"implicit",
        "smtp_host":"smtp.gmail.com","smtp_port":465,"smtp_tls":"implicit", "username":"alice@example.test","password":"",
        "gmail_oauth":{"client_id":"development","refresh_token":"private-refresh-fixture"}
    })).unwrap();
    let response = router(app.clone())
        .oneshot(req(
            "POST",
            "/api/accounts",
            "alice-token",
            true,
            serde_json::to_string(&input).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let account = app.store.connect(&alice, &input).await.unwrap();
    let encrypted: Vec<u8> = sqlx::query_scalar("SELECT credential FROM accounts WHERE id=?")
        .bind(&account.id)
        .fetch_one(app.store.control())
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&encrypted).contains("private-refresh-fixture"));
    let a = app.store.authorize(&alice, &account.id).await.unwrap();
    assert_eq!(
        app.store
            .credential(&a)
            .await
            .unwrap()
            .gmail_oauth
            .unwrap()
            .refresh_token,
        "private-refresh-fixture"
    );
    assert!(
        !serde_json::to_string(&app.store.accounts(&alice).await.unwrap())
            .unwrap()
            .contains("refresh")
    );
    input.gmail_oauth = None;
    assert!(app.store.connect(&alice, &input).await.is_err());
}
