//! Disposable browser -> real handlers/SQLite/executor -> TLS SMTP receiver.
use super::*;
use crate::unsubscribe::{Action, Confirmation};
use crate::{App, Identity};
use axum::{extract::Path, Json};
#[path = "../../mail-provider/tests/common/mod.rs"]
mod wire;
use wire::{line, reply, smtp_login, Fixture};

struct Sending {
    base: Arc<Controlled>,
    smtp: p::smtp::SmtpProvider,
    sends: AtomicUsize,
    filings: AtomicUsize,
}
#[async_trait]
impl MailProvider for Sending {
    fn account_id(&self) -> &str {
        self.base.account_id()
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        self.base.probe().await
    }
    async fn scan(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        self.base.scan(f, q, t, n).await
    }
    async fn search(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        self.base.search(f, q, t, n).await
    }
    async fn fetch(&self, r: &p::MessageRef, n: u32) -> p::Result<Vec<u8>> {
        self.base.fetch(r, n).await
    }
    async fn mutate(&self, r: &p::MessageRef, m: p::Mutation) -> p::Result<p::MutationResult> {
        self.base.mutate(r, m).await
    }
    async fn wait_for_change(&self, f: &str, d: Duration) -> p::Result<bool> {
        self.base.wait_for_change(f, d).await
    }
    async fn submit(
        &self,
        m: &p::mime::PreparedMessage,
        gate: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.smtp.submit(m, || gate.begin_data()).await
    }
    async fn reconcile_sent(&self, _: &str, raw: &[u8]) -> p::Result<p::SentOutcome> {
        let mail = mail_parser::MessageParser::default().parse(raw).unwrap();
        assert!(mail.message_id().is_some());
        self.filings.fetch_add(1, Ordering::SeqCst);
        Ok(p::SentOutcome::Appended)
    }
}
async fn sending() -> (
    tempfile::TempDir,
    App,
    String,
    AuthorizedAccount,
    Arc<Sending>,
    Fixture,
) {
    let (dir, app, user, a, base) = controlled().await;
    let fixture = Fixture::new().await;
    let mut config = fixture.config(p::Security::Tls);
    config.account_id = a.id().into();
    config.primary_address = app.store.credential(&a).await.unwrap().email;
    let smtp = p::smtp::SmtpProvider::new(config, wire::credentials())
        .unwrap()
        .with_root(
            lettre::transport::smtp::client::Certificate::from_pem(fixture.pem.as_bytes()).unwrap(),
        )
        .unwrap();
    let provider = Arc::new(Sending {
        base,
        smtp,
        sends: AtomicUsize::new(0),
        filings: AtomicUsize::new(0),
    });
    app.mail
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    (dir, app, user, a, provider, fixture)
}
async fn seed(app: &App, a: &AuthorizedAccount, id: &str) -> Message {
    let mut m = app
        .mail
        .snapshot_page(&app.store, a, "INBOX", None)
        .await
        .unwrap()
        .rows[0]
        .clone();
    m.id = id.into();
    m.subject = format!("Synthetic Bigbank {id}");
    m.sender.name = Some("Bigbank fixture".into());
    let m = app.store.upsert_message(a, m).await.unwrap();
    let content: MessageContent = serde_json::from_value(serde_json::json!({"message":m,"text":"Synthetic newsletter. No original content or attachments.","html":"","attachments":[],"message_id":null,"references":[],"cc":[]})).unwrap();
    app.store
        .cache_content(a, &content, 1024 * 1024)
        .await
        .unwrap();
    crate::unsubscribe::remember(&app.store, a, id, b"List-Unsubscribe: <mailto:listunsubscribe-synthetic@email.example.org?subject=Leave%2Bnow&body=Stop%0D%0Aplease>\r\n\r\nPRIVATE ORIGINAL MUST NOT LEAK").await.unwrap();
    m
}
async fn receiver(fixture: Fixture, expected_from: String, mode: &'static str) -> String {
    let mut w = fixture.accept(false, true).await;
    smtp_login(&mut w, false).await;
    assert!(line(&mut w)
        .await
        .starts_with(&format!("MAIL FROM:<{expected_from}>")));
    reply(&mut w, "250 sender ok\r\n").await;
    assert!(line(&mut w)
        .await
        .starts_with("RCPT TO:<listunsubscribe-synthetic@email.example.org>"));
    if mode == "reject" {
        reply(&mut w, "550 rejected\r\n").await;
        return String::new();
    }
    reply(&mut w, "250 recipient ok\r\n").await;
    assert_eq!(line(&mut w).await, "DATA\r\n");
    reply(&mut w, "354 send body\r\n").await;
    let mut raw = String::new();
    loop {
        let l = line(&mut w).await;
        assert!(!l.is_empty());
        if l == ".\r\n" {
            break;
        }
        raw.push_str(&l);
    }
    let mail = mail_parser::MessageParser::default()
        .parse(raw.as_bytes())
        .unwrap();
    assert_eq!(
        mail.from().unwrap().first().unwrap().address(),
        Some(expected_from.as_str())
    );
    assert_eq!(mail.to().unwrap().iter().count(), 1);
    assert_eq!(mail.subject(), Some("Leave+now"));
    assert_eq!(mail.body_text(0).unwrap().trim(), "Stop\r\nplease");
    assert!(mail.cc().is_none() && mail.bcc().is_none());
    assert_eq!(mail.attachments().count(), 0);
    assert!(!raw.contains("PRIVATE ORIGINAL"));
    if mode != "unknown" {
        reply(&mut w, "250 queued synthetic\r\n").await;
    }
    raw
}
#[tokio::test]
async fn unsubscribe_smtp_delivery_outcomes_are_read_back_without_retransmission() {
    for (mode, outcome) in [
        ("accepted", "request_accepted"),
        ("unknown", "result_unknown"),
        ("reject", "not_sent"),
    ] {
        let (_dir, app, user, a, provider, fixture) = sending().await;
        seed(&app, &a, "smtp").await;
        let receiver = tokio::spawn(receiver(
            fixture,
            app.store.credential(&a).await.unwrap().email,
            mode,
        ));
        let action = Action::Mailto {
            to: "listunsubscribe-synthetic@email.example.org".into(),
            subject: "Leave+now".into(),
            body: "Stop\r\nplease".into(),
        };
        let invoke = || {
            crate::unsubscribe::execute(
                State(app.clone()),
                Extension(Identity {
                    user: user.clone(),
                    csrf: "fixture".into(),
                    token_hash: "fixture".into(),
                }),
                Path((a.id().into(), "smtp".into())),
                Json(
                    serde_json::from_value::<Confirmation>(
                        serde_json::json!({"expected_action":action}),
                    )
                    .unwrap(),
                ),
            )
        };
        let (one, two) = tokio::join!(invoke(), invoke());
        assert_eq!(one.unwrap().status(), StatusCode::ACCEPTED);
        assert_eq!(two.unwrap().status(), StatusCode::ACCEPTED);
        crate::executor::tick(&app, &a).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), receiver)
            .await
            .unwrap()
            .unwrap();
        let info = crate::unsubscribe::info(
            State(app.clone()),
            Extension(Identity {
                user: user.clone(),
                csrf: "fixture".into(),
                token_hash: "fixture".into(),
            }),
            Path((a.id().into(), "smtp".into())),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(info["attempt_status"], outcome);
        let replay = invoke().await;
        assert_eq!(
            replay.map(|r| r.status()).unwrap_or(StatusCode::CONFLICT),
            if mode == "accepted" {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::CONFLICT
            }
        );
        crate::executor::tick(&app, &a).await.unwrap();
        assert_eq!(provider.sends.load(Ordering::SeqCst), 1);
        assert_eq!(
            provider.filings.load(Ordering::SeqCst),
            usize::from(mode == "accepted")
        );
        assert!(app.store.drafts(&a).await.unwrap().is_empty());
        println!("SMTP {mode}: one immutable send, durable {outcome}, no replay");
    }
}

#[tokio::test]
async fn unsubscribe_restart_recovers_in_data_as_unknown_never_scheduled() {
    let (dir, app, user, a, provider, _fixture) = sending().await;
    seed(&app, &a, "crash").await;
    let input = || {
        serde_json::from_value::<Confirmation>(serde_json::json!({"expected_action":{"kind":"mailto","to":"listunsubscribe-synthetic@email.example.org","subject":"Leave+now","body":"Stop\r\nplease"}})).unwrap()
    };
    let identity = Identity {
        user: user.clone(),
        csrf: "fixture".into(),
        token_hash: "fixture".into(),
    };
    crate::unsubscribe::execute(
        State(app.clone()),
        Extension(identity.clone()),
        Path((a.id().into(), "crash".into())),
        Json(input()),
    )
    .await
    .unwrap();
    let s = app
        .store
        .due_submission(&a, "9999-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .unwrap();
    app.store
        .transition(
            &a,
            &s.id,
            SubmissionState::Claimed,
            SubmissionState::Submitting,
        )
        .await
        .unwrap();
    // A new Store and new account authorization read the persisted intent.
    let reopened = dakia_storage::Store::open(dir.path(), [29; 32])
        .await
        .unwrap();
    let scope = reopened.authorize(&user, a.id()).await.unwrap();
    reopened
        .recover_expired(&scope, "9999-01-01T00:00:00.000Z")
        .await
        .unwrap();
    let app = App {
        store: reopened,
        config: app.config.clone(),
        mail: app.mail.clone(),
    };
    let info = crate::unsubscribe::info(
        State(app.clone()),
        Extension(identity.clone()),
        Path((scope.id().into(), "crash".into())),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(info["attempt_status"], "result_unknown");
    assert!(crate::unsubscribe::execute(
        State(app.clone()),
        Extension(identity),
        Path((scope.id().into(), "crash".into())),
        Json(input())
    )
    .await
    .is_err());
    crate::executor::tick(&app, &scope).await.unwrap();
    assert_eq!(provider.sends.load(Ordering::SeqCst), 0);
    assert_eq!(app.store.submissions(&scope).await.unwrap().len(), 1);
    assert!(app
        .store
        .due_submission(&scope, "9999-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "loopback browser fixture only"]
async fn browser_unsubscribe_fixture() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let (_dir, mut app, user, a, provider, fixture) = sending().await;
    seed(&app, &a, "one-click-email").await;
    seed(&app, &a, "one-click-http").await;
    app.store
        .put_provider_document(
            &a,
            "unsubscribe-v1:one-click-http",
            r#"{"kind":"one_click","url":"https://newsletter.example.org/browser-synthetic"}"#,
        )
        .await
        .unwrap();
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_url = url::Url::parse(&format!(
        "http://{}/unsubscribe",
        http_listener.local_addr().unwrap()
    ))
    .unwrap();
    let http_count = Arc::new(AtomicUsize::new(0));
    let count = http_count.clone();
    let endpoint = axum::Router::new().route(
        "/unsubscribe",
        axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
            let count = count.clone();
            async move {
                assert_eq!(
                    count.fetch_add(1, Ordering::SeqCst),
                    0,
                    "no duplicate HTTP request"
                );
                assert_eq!(headers["content-type"], "application/x-www-form-urlencoded");
                assert!(
                    !headers.contains_key("cookie")
                        && !headers.contains_key("authorization")
                        && !headers.contains_key("referer")
                );
                assert_eq!(body, "List-Unsubscribe=One-Click");
                StatusCode::NO_CONTENT
            }
        }),
    );
    crate::unsubscribe::BROWSER_HTTP
        .set((
            reqwest::Client::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            http_url,
        ))
        .unwrap();
    tokio::spawn(async move {
        axum::serve(http_listener, endpoint).await.unwrap();
    });
    let receiver = tokio::spawn(receiver(
        fixture,
        app.store.credential(&a).await.unwrap().email,
        "accepted",
    ));
    app.config = Arc::new(crate::Config {
        origin: "http://127.0.0.1:4327".into(),
        issuer: "https://id.example.invalid".into(),
        client_id: "fixture".into(),
        client_secret: "fixture-only".into(),
        web_root: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/web/dist")
            .to_string_lossy()
            .into(),
    });
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(b"release-browser-fixture")),
        )
        .bind(user)
        .bind("release-browser-csrf")
        .bind(4102444800_i64)
        .execute(app.store.control())
        .await
        .unwrap();
    let worker_app = app.clone();
    let worker_a = a.clone();
    let worker = tokio::spawn(async move {
        loop {
            crate::executor::tick(&worker_app, &worker_a).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let verified = Arc::new(AtomicBool::new(false));
    let received = verified.clone();
    tokio::spawn(async move {
        receiver.await.unwrap();
        received.store(true, Ordering::SeqCst);
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:4327")
        .await
        .unwrap();
    println!("Disposable unsubscribe browser fixture ready");
    let router = crate::router(app).route("/fixture-receipts", axum::routing::get(move || {
        let provider = provider.clone(); let verified = verified.clone(); let http_count = http_count.clone();
        async move { Json(serde_json::json!({"smtp_verified": verified.load(Ordering::SeqCst), "smtp_sends": provider.sends.load(Ordering::SeqCst), "http_requests":http_count.load(Ordering::SeqCst)})) }
    }));
    axum::serve(listener, router).await.unwrap();
    worker.abort();
}
