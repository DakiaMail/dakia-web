use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn token_source(token_url: String) -> (tempfile::TempDir, MicrosoftToken) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path(), [7; 32]).await.unwrap();
    let user = store
        .user("https://fixture.invalid", "fixture")
        .await
        .unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({
        "email":"fixture@example.test","display_name":"Fixture","provider":"microsoft",
        "imap_host":"graph.microsoft.com","imap_port":443,"imap_tls":"implicit",
        "smtp_host":"graph.microsoft.com","smtp_port":443,"smtp_tls":"implicit",
        "username":"fixture@example.test","password":"",
        "microsoft_oauth":{"client_id":"fixture-client","tenant":"common","subject":"fixture-subject","refresh_token":"fixture-refresh"}
    })).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let scope = store.authorize(&user, &account.id).await.unwrap();
    let mut source = MicrosoftToken::new(
        MicrosoftConfig {
            client_id: "fixture-client".into(),
            client_secret: "fixture-secret".into(),
            tenant: "common".into(),
        },
        input.microsoft_oauth.unwrap(),
        store,
        scope,
    );
    source.token_url = token_url;
    (directory, source)
}
#[tokio::test]
async fn renewal_is_single_flight_cached_and_rotates_encrypted_refresh_tokens_before_expiry() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let server = Router::new().route("/token",post(move |axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String,String>>| {
        let counter = counter.clone();
        async move {
            let n = counter.fetch_add(1,Ordering::SeqCst);
            assert_eq!(form["grant_type"],"refresh_token");
            assert_eq!(form["refresh_token"],if n == 0 {"fixture-refresh"} else {"rotated-0"});
            assert_eq!(form["client_id"],"fixture-client");
            assert_eq!(form["client_secret"],"fixture-secret");
            assert!(form["scope"].contains("offline_access"));
            Json(serde_json::json!({"access_token":format!("access-{n}"),"refresh_token":format!("rotated-{n}"),"expires_in":3600,"token_type":"Bearer","scope":"User.Read Mail.ReadWrite Mail.Send"}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let (_directory, source) = token_source(format!("http://{address}/token")).await;
    let (a, b) = tokio::join!(source.access_token(), source.access_token());
    assert_eq!(a.unwrap(), "access-0");
    assert_eq!(b.unwrap(), "access-0");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        source
            .store
            .credential(&source.account)
            .await
            .unwrap()
            .microsoft_oauth
            .unwrap()
            .refresh_token,
        "rotated-0"
    );
    *source.cache.lock().await = Some((Instant::now() - Duration::from_secs(1), "expired".into()));
    assert_eq!(source.access_token().await.unwrap(), "access-1");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        source
            .store
            .credential(&source.account)
            .await
            .unwrap()
            .microsoft_oauth
            .unwrap()
            .refresh_token,
        "rotated-1"
    );
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT credential FROM accounts WHERE id=?")
        .bind(source.account.id())
        .fetch_one(source.store.control())
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("rotated-1"));
    task.abort();
}
#[tokio::test]
async fn revoked_refresh_grants_require_reconnection_without_exposing_provider_details() {
    let server = Router::new().route("/token",post(|| async {
        (StatusCode::BAD_REQUEST,Json(serde_json::json!({"error":"invalid_grant","error_description":"private mailbox detail"})))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let (_directory, source) = token_source(format!("http://{address}/token")).await;
    let error = source.access_token().await.unwrap_err();
    assert_eq!(error.kind, p::ErrorKind::Authentication);
    assert_eq!(error.message, "Microsoft token renewal failed");
    assert!(source.cache.lock().await.is_none());
    task.abort();
}
#[tokio::test]
async fn authorization_code_profile_and_persistence_use_the_mailbox_and_reconnect_the_same_subject()
{
    use axum::extract::Form;
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path(), [7; 32]).await.unwrap();
    let user = store
        .user("https://fixture.invalid", "owner")
        .await
        .unwrap();
    let identity = Identity {
        user: user.clone(),
        csrf: "fixture-csrf".into(),
        token_hash: hash("fixture-session"),
    };
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(&identity.token_hash)
        .bind(&user)
        .bind(&identity.csrf)
        .bind(4102444800i64)
        .execute(store.control())
        .await
        .unwrap();
    let app = App {
        store,
        mail: crate::mail::MailRuntime::default().with_microsoft(Some(MicrosoftConfig {
            client_id: "fixture-client".into(),
            client_secret: "fixture-secret".into(),
            tenant: "common".into(),
        })),
        config: Arc::new(Config {
            origin: "https://mail.example.invalid".into(),
            issuer: "https://fixture.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture".into(),
            web_root: directory.path().to_string_lossy().into(),
        }),
    };
    let profile = Arc::new(std::sync::Mutex::new(
        serde_json::json!({"id":"fixture-subject","mail":"mailbox@example.test","userPrincipalName":"login@tenant.test"}),
    ));
    let permission = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let profile_copy = profile.clone();
    let permission_copy = permission.clone();
    let tokens = Arc::new(AtomicUsize::new(0));
    let token_count = tokens.clone();
    let server=Router::new().route("/token",post(move|Form(form):Form<std::collections::HashMap<String,String>>|{
        let permission=permission_copy.clone();let calls=token_count.clone();async move {
            calls.fetch_add(1,Ordering::SeqCst);
            assert_eq!(form["grant_type"],"authorization_code");assert_eq!(form["code"],"fixture-code");
            assert_eq!(form["client_secret"],"fixture-secret");assert!(form["code_verifier"].len()>=43);
            assert_eq!(form["redirect_uri"],"https://mail.example.invalid/api/accounts/microsoft/callback");
            Json(serde_json::json!({"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600,"token_type":"Bearer",
                "scope":if permission.load(Ordering::SeqCst) {"User.Read Mail.ReadWrite Mail.Send"} else {"User.Read Mail.Read"}}))
        }
    })).route("/me",get(move|headers:axum::http::HeaderMap|{
        let profile=profile_copy.clone();async move {
            assert_eq!(headers["authorization"],"Bearer fixture-access");Json(profile.lock().unwrap().clone())
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    async fn flow(app: &App, id: &Identity, email: &str, account: Option<String>) -> Callback {
        let Json(body) = start(
            State(app.clone()),
            Extension(id.clone()),
            Json(Start {
                email: email.into(),
                display_name: "Fixture".into(),
                account_id: account,
            }),
        )
        .await
        .unwrap();
        let url = url::Url::parse(body["url"].as_str().unwrap()).unwrap();
        Callback {
            state: url
                .query_pairs()
                .find(|(key, _)| key == "state")
                .unwrap()
                .1
                .into_owned(),
            code: Some("fixture-code".into()),
            error: None,
        }
    }
    let token_url = format!("http://{address}/token");
    let profile_url = format!("http://{address}/me");
    let callback = flow(&app, &identity, "login@tenant.test", None).await;
    let a = authorize(&app, &identity, callback, &token_url, &profile_url)
        .await
        .unwrap();
    let credential = app.store.credential(&a).await.unwrap();
    assert_eq!(credential.email, "mailbox@example.test");
    assert_eq!(credential.username, "mailbox@example.test");
    assert_eq!(
        crate::mail::configuration(&a, &credential).primary_address,
        "mailbox@example.test"
    );
    assert_eq!(
        app.store.accounts(&user).await.unwrap()[0].email,
        "mailbox@example.test"
    );
    let original = a.id().to_owned();
    // A primary SMTP rename must not strand an otherwise valid connection or replace its account id.
    profile.lock().unwrap()["mail"] = serde_json::json!("renamed@example.test");
    let callback = flow(
        &app,
        &identity,
        "mailbox@example.test",
        Some(original.clone()),
    )
    .await;
    let reconnected = authorize(&app, &identity, callback, &token_url, &profile_url)
        .await
        .unwrap();
    assert_eq!(reconnected.id(), original);
    assert_eq!(
        app.store.credential(&reconnected).await.unwrap().email,
        "renamed@example.test"
    );
    profile.lock().unwrap()["id"] = serde_json::json!("wrong-subject");
    let callback = flow(
        &app,
        &identity,
        "renamed@example.test",
        Some(original.clone()),
    )
    .await;
    assert_eq!(
        authorize(&app, &identity, callback, &token_url, &profile_url)
            .await
            .err()
            .unwrap()
            .1,
        "microsoft_account_mismatch"
    );
    assert_eq!(
        app.store
            .credential(&reconnected)
            .await
            .unwrap()
            .microsoft_oauth
            .unwrap()
            .subject,
        "fixture-subject"
    );
    permission.store(false, Ordering::SeqCst);
    let callback = flow(&app, &identity, "login@tenant.test", None).await;
    assert_eq!(
        authorize(&app, &identity, callback, &token_url, &profile_url)
            .await
            .err()
            .unwrap()
            .1,
        "microsoft_permission_required"
    );
    assert_eq!(app.store.accounts(&user).await.unwrap().len(), 1);
    assert_eq!(tokens.load(Ordering::SeqCst), 4);
    task.abort();
}
#[test]
fn tenant_and_mail_grant_validation_rejects_injection_missing_scopes_and_invalid_tokens() {
    for tenant in [
        "common",
        "organizations",
        "consumers",
        "9188040d-6c67-4c5b-b112-36a304b66dad",
    ] {
        assert!(valid_tenant(tenant));
    }
    for tenant in [
        "",
        "../common",
        "common?client_id=other",
        "https://attacker.test",
    ] {
        assert!(!valid_tenant(tenant));
    }
    let mut tokens: Tokens = serde_json::from_value(serde_json::json!({"access_token":"fixture","expires_in":3600,"token_type":"Bearer","scope":"https://graph.microsoft.com/User.Read Mail.ReadWrite Mail.Send"})).unwrap();
    assert!(valid_tokens(&tokens));
    assert!(mail_permissions(&tokens));
    tokens.scope = None;
    assert!(mail_permissions(&tokens));
    tokens.scope = Some("User.Read Mail.Read".into());
    assert!(!mail_permissions(&tokens));
    tokens.access_token = "fixture\r\ninjection".into();
    assert!(!valid_tokens(&tokens));
    tokens.access_token = "fixture".into();
    tokens.expires_in = 60;
    assert!(!valid_tokens(&tokens));
}
