use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn google_reconnect_replaces_expired_grant_only_after_valid_callback() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path(), [7; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let identity = Identity {
        user: user.clone(),
        csrf: "fixture".into(),
        token_hash: hash("fixture-session"),
    };
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(&identity.token_hash)
        .bind(&user)
        .bind(&identity.csrf)
        .bind(4102444800_i64)
        .execute(store.sqlite_control_for_test())
        .await
        .unwrap();
    let app = App {
        store,
        mail: crate::mail::MailRuntime::default().with_gmail(Some(GmailConfig {
            client_id: "fixture-client".into(),
            client_secret: "fixture-secret".into(),
        })),
        config: Arc::new(Config {
            origin: "https://mail.example.invalid".into(),
            issuer: "https://fixture.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture".into(),
            web_root: directory.path().to_string_lossy().into(),
        }),
    };
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({
        "email":"fixture@example.test", "display_name":"Fixture", "provider":"gmail",
        "imap_host":"imap.gmail.com", "imap_port":993, "imap_tls":"implicit",
        "smtp_host":"smtp.gmail.com", "smtp_port":465, "smtp_tls":"implicit",
        "username":"fixture@example.test", "password":"",
        "gmail_oauth":{"client_id":"fixture-client","refresh_token":"expired-fixture"}
    }))
    .unwrap();
    let account = app.store.connect(&user, &input).await.unwrap();
    let a = app.store.authorize(&user, &account.id).await.unwrap();
    app.store
        .account_state(&a, AccountState::Reconnect)
        .await
        .unwrap();
    app.store
        .put_provider_document(&a, "fixture-data", "keep")
        .await
        .unwrap();
    let old_provider = app.mail.provider(&app.store, &a).await.unwrap();
    let mode = Arc::new(AtomicUsize::new(0));
    let token_mode = mode.clone();
    let profile_mode = mode.clone();
    let server = Router::new()
        .route("/token", post(move |axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String,String>>| {
            let mode = token_mode.clone();
            async move {
                assert_eq!(form["grant_type"], "authorization_code");
                assert_eq!(form["code"], "fixture-code");
                assert_eq!(form["client_secret"], "fixture-secret");
                assert!(form["code_verifier"].len() >= 43);
                assert_eq!(form["redirect_uri"], "https://mail.example.invalid/api/accounts/gmail/callback");
                let mut tokens = serde_json::json!({"access_token":"fixture-access","refresh_token":"replacement-fixture","expires_in":3600,"token_type":"Bearer","scope":format!("openid email {MAIL_SCOPE}")});
                match mode.load(Ordering::SeqCst) {
                    0 => { tokens["scope"] = "openid email".into(); }
                    1 => { tokens.as_object_mut().unwrap().remove("refresh_token"); }
                    2 => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"invalid_grant","error_description":"private provider detail"}))),
                    _ => {}
                }
                (StatusCode::OK, Json(tokens))
            }
        }))
        .route("/profile", get(move |headers: HeaderMap| {
            let mode = profile_mode.clone();
            async move {
                assert_eq!(headers["authorization"], "Bearer fixture-access");
                if mode.load(Ordering::SeqCst) == 3 {
                    return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":"private provider detail"})));
                }
                (StatusCode::OK, Json(serde_json::json!({"email":"fixture@example.test","email_verified":true})))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let token_url = format!("http://{address}/token");
    let profile_url = format!("http://{address}/profile");
    for (n, expected) in [
        "gmail_permission_required",
        "google_offline_access_required",
        "google_token_exchange_failed",
        "google_profile_failed",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        mode.store(n, Ordering::SeqCst);
        let Json(body) = start(
            State(app.clone()),
            Extension(identity.clone()),
            Json(Start {
                email: input.email.clone(),
                display_name: input.display_name.clone(),
                account_id: Some(account.id.clone()),
            }),
        )
        .await
        .unwrap();
        let url = url::Url::parse(body["url"].as_str().unwrap()).unwrap();
        assert!(url
            .query_pairs()
            .any(|(key, value)| key == "prompt" && value == "consent"));
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let result = authorize(
            &app,
            &identity,
            Callback {
                state: state.clone(),
                code: Some("fixture-code".into()),
                error: None,
            },
            &token_url,
            &profile_url,
        )
        .await;
        if expected.is_empty() {
            assert_eq!(result.unwrap().id(), a.id());
        } else {
            assert_eq!(result.unwrap_err().1, expected);
            assert_eq!(
                app.store
                    .credential(&a)
                    .await
                    .unwrap()
                    .gmail_oauth
                    .unwrap()
                    .refresh_token,
                "expired-fixture"
            );
            assert_eq!(
                app.store.accounts(&user).await.unwrap()[0].state,
                AccountState::Reconnect
            );
        }
        let replay = authorize(
            &app,
            &identity,
            Callback {
                state,
                code: Some("fixture-code".into()),
                error: None,
            },
            &token_url,
            &profile_url,
        )
        .await;
        assert_eq!(replay.unwrap_err().1, "google_sign_in_expired");
    }
    assert_eq!(app.store.accounts(&user).await.unwrap().len(), 1);
    assert_eq!(
        app.store
            .credential(&a)
            .await
            .unwrap()
            .gmail_oauth
            .unwrap()
            .refresh_token,
        "replacement-fixture"
    );
    assert_eq!(
        app.store
            .provider_document(&a, "fixture-data")
            .await
            .unwrap()
            .as_deref(),
        Some("keep")
    );
    let new_provider = app.mail.provider(&app.store, &a).await.unwrap();
    assert!(!Arc::ptr_eq(&old_provider, &new_provider));
    let reopened = Store::open(directory.path(), [7; 32]).await.unwrap();
    let restored = reopened.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        reopened
            .credential(&restored)
            .await
            .unwrap()
            .gmail_oauth
            .unwrap()
            .refresh_token,
        "replacement-fixture"
    );
    assert_eq!(reopened.accounts(&user).await.unwrap().len(), 1);
    assert_eq!(
        reopened
            .provider_document(&restored, "fixture-data")
            .await
            .unwrap()
            .as_deref(),
        Some("keep")
    );
    task.abort();
}
