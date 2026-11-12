use super::*;
#[path = "graph/parity_tests.rs"]
mod parity;
use axum::{
    body::Bytes, extract::Request, http::HeaderMap, response::IntoResponse, routing::any, Router,
};
use std::sync::atomic::AtomicUsize;

struct Token;
#[async_trait]
impl AccessToken for Token {
    async fn access_token(&self) -> Result<String> {
        Ok("fixture-access".into())
    }
}
async fn fixture(
    handler: impl Fn(Method, axum::http::Uri, HeaderMap, Bytes) -> axum::response::Response
        + Send
        + Sync
        + 'static,
) -> (GraphProvider, tokio::task::JoinHandle<()>) {
    let handler = Arc::new(handler);
    let app = Router::new().fallback(any(move |request: Request| {
        let handler = handler.clone();
        async move {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024).await.unwrap();
            handler(parts.method, parts.uri, parts.headers, bytes)
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut provider = GraphProvider::new(
        "fixture-account".into(),
        Arc::new(Token),
        FolderMapping::default(),
    );
    provider.base = Url::parse(&format!("http://{address}/v1.0/")).unwrap();
    (provider, task)
}
fn json(value: serde_json::Value) -> axum::response::Response {
    axum::Json(value).into_response()
}
fn message(id: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"isRead":false,"flag":{"flagStatus":"flagged"},"receivedDateTime":"2026-10-29T12:00:00Z",
        "internetMessageHeaders":[{"name":"From","value":"Sender <sender@example.test>"},{"name":"To","value":"fixture@example.test"},
            {"name":"Subject","value":"A fixture"},{"name":"Message-ID","value":"<fixture@example.test>"},{"name":"X_Custom","value":"Allowed"}]})
}
#[tokio::test]
async fn paginated_sync_preserves_opaque_ids_and_scopes_continuations_and_content() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (provider,task) = fixture(move |method,uri,headers,_| {
        count.fetch_add(1,Ordering::SeqCst);
        assert_eq!(headers["authorization"],"Bearer fixture-access");
        assert_eq!(headers["prefer"],"IdType=\"ImmutableId\"");
        assert_eq!(method,Method::GET);
        if uri.path() == "/v1.0/me/messages" { return json(serde_json::json!({"value":[]})); }
        if uri.path() == "/v1.0/me/mailFolders/inbox" {
            return json(serde_json::json!({"id":"inbox-id","displayName":"Localized inbox","childFolderCount":0}));
        }
        if uri.path() == "/v1.0/me/mailFolders" {
            return json(serde_json::json!({"value":[{"id":"inbox-id","displayName":"Localized inbox","childFolderCount":0}]}));
        }
        if uri.path() == "/v1.0/me/mailFolders/inbox-id/messages" {
            if uri.query().unwrap_or("").contains("page=2") {
                return json(serde_json::json!({"value":[message("opaque/second+id=")]}));
            }
            return json(serde_json::json!({"value":[message("opaque/first+id=")],"@odata.nextLink":"me/mailFolders/inbox-id/messages?page=2"}));
        }
        if uri.path().ends_with("/$value") { return "From: sender@example.test\r\nSubject: Body\r\n\r\nHello".into_response(); }
        StatusCode::NOT_FOUND.into_response()
    }).await;
    let probe = provider.probe().await.unwrap();
    assert_eq!(probe.folders[0].name, "INBOX");
    assert_eq!(probe.folders[0].attributes, vec!["Inbox"]);
    let page = provider
        .scan("INBOX", &Search::default(), None, 1)
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(
        page.rows[0].reference.graph_id.as_deref(),
        Some("opaque/first+id=")
    );
    assert!(
        matches!(&page.rows[0].identity,Identity::Graph { message_id,.. } if message_id == "opaque/first+id=")
    );
    assert!(page.rows[0].starred);
    assert_eq!(page.rows[0].internal_date, "29-Oct-2026 12:00:00 +0000");
    let reference = &page.rows[0].reference;
    assert!(provider
        .fetch(reference, 1024)
        .await
        .unwrap()
        .ends_with(b"Hello"));
    assert!(provider.fetch(reference, 3).await.is_err());
    let next = provider
        .scan("INBOX", &Search::default(), page.next.as_ref(), 1)
        .await
        .unwrap();
    assert_eq!(
        next.rows[0].reference.graph_id.as_deref(),
        Some("opaque/second+id=")
    );
    assert!(next.next.is_none());
    let before = calls.load(Ordering::SeqCst);
    let mut token = page.next.unwrap();
    token.graph_next = Some("https://attacker.example/v1.0/me/messages".into());
    assert!(provider
        .scan("INBOX", &Search::default(), Some(&token), 1)
        .await
        .is_err());
    token.graph_next = Some("me/mailFolders/another-folder/messages".into());
    assert!(provider
        .scan("INBOX", &Search::default(), Some(&token), 1)
        .await
        .is_err());
    token.account_id = "other-account".into();
    assert!(provider
        .scan("INBOX", &Search::default(), Some(&token), 1)
        .await
        .is_err());
    let mut foreign = reference.clone();
    foreign.account_id = "other-account".into();
    assert!(provider.fetch(&foreign, 1024).await.is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "no bearer token may leave for an invalid reference"
    );
    provider.retire();
    assert!(provider.fetch(reference, 1024).await.is_err());
    task.abort();
}
#[tokio::test]
async fn nested_folders_search_and_move_keep_provider_identity_and_uncertain_writes_are_not_replayed(
) {
    let writes = Arc::new(AtomicUsize::new(0));
    let count = writes.clone();
    let (provider,task) = fixture(move |method,uri,_,body| {
        if method == Method::POST {
            assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["destinationId"],"archive-id");
            let n = count.fetch_add(1,Ordering::SeqCst);
            return if n == 0 { json(serde_json::json!({"id":"moved-id"})) } else { StatusCode::BAD_GATEWAY.into_response() };
        }
        let f = |id:&str,name:&str,children:u32| serde_json::json!({"id":id,"displayName":name,"childFolderCount":children});
        match uri.path() {
            "/v1.0/me/mailFolders/inbox" => json(f("inbox-id","Inbox",0)),
            "/v1.0/me/mailFolders/archive" => json(f("archive-id","Archive",0)),
            "/v1.0/me/mailFolders" => json(serde_json::json!({"value":[f("inbox-id","Inbox",0),f("archive-id","Archive",0),f("parent-id","Projects",1)]})),
            "/v1.0/me/mailFolders/parent-id/childFolders" => json(serde_json::json!({"value":[f("child-id","A/B",0)]})),
            "/v1.0/me/mailFolders/inbox-id/messages" => json(serde_json::json!({"value":[message("original-id")]})),
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    }).await;
    assert!(provider
        .folders()
        .await
        .unwrap()
        .iter()
        .any(|f| f.name == "Folders/Projects/A%2FB"));
    let query = Search {
        from: Some("SENDER@example.test".into()),
        starred: Some(true),
        since: Some("29-Oct-2026".into()),
        ..Default::default()
    };
    let page = provider.search("INBOX", &query, None, 10).await.unwrap();
    assert_eq!(page.rows.len(), 1);
    let result = provider
        .mutate(&page.rows[0].reference, Mutation::Archive)
        .await
        .unwrap();
    assert_eq!(result.destination.as_deref(), Some("Archive"));
    assert_eq!(
        result.destination_reference.unwrap().graph_id.as_deref(),
        Some("moved-id")
    );
    assert_eq!(
        provider
            .mutate(&page.rows[0].reference, Mutation::Archive)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Uncertain
    );
    assert_eq!(
        writes.load(Ordering::SeqCst),
        2,
        "uncertain writes must not be retried"
    );
    task.abort();
}
struct Gate(AtomicUsize);
#[async_trait]
impl SubmissionGate for Gate {
    async fn begin_data(&self) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[tokio::test]
async fn mime_sending_preserves_bcc_and_crosses_the_durable_gate_once_without_duplicate_sent_copy()
{
    let writes = Arc::new(AtomicUsize::new(0));
    let count = writes.clone();
    let (provider, task) = fixture(move |method, uri, headers, body| {
        assert_eq!(method, Method::POST);
        assert_eq!(uri.path(), "/v1.0/me/sendMail");
        assert_eq!(headers["content-type"], "text/plain");
        let raw = STANDARD.decode(body).unwrap();
        let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(
            parsed.bcc().unwrap().first().unwrap().address(),
            Some("hidden@example.test")
        );
        assert_eq!(parsed.subject(), Some("Fixture"));
        let n = count.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            StatusCode::ACCEPTED.into_response()
        } else {
            StatusCode::BAD_GATEWAY.into_response()
        }
    })
    .await;
    let endpoint = Endpoint {
        host: "unused".into(),
        port: 443,
        security: Security::Tls,
    };
    let config = Config {
        account_id: "fixture-account".into(),
        primary_address: "fixture@example.test".into(),
        kind: ProviderKind::Imap,
        imap: endpoint.clone(),
        smtp: endpoint,
        timeout: Duration::from_secs(1),
        full_imap_exposure_confirmed: false,
        folders: Default::default(),
    };
    let mut prepared = mime::assemble(
        &config,
        mime::Compose {
            to: vec!["visible@example.test".into()],
            cc: vec![],
            bcc: vec!["hidden@example.test".into()],
            subject: "Fixture".into(),
            text: "Hello".into(),
            html: None,
            message_id: "<fixture@example.test>".into(),
            in_reply_to: None,
            references: None,
            attachments: vec![],
        },
        25 * 1024 * 1024,
    )
    .unwrap();
    let gate = Gate(AtomicUsize::new(0));
    assert!(matches!(
        provider.submit(&prepared, &gate).await,
        SubmissionOutcome::Accepted { .. }
    ));
    assert_eq!(gate.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider
            .reconcile_sent("fixture@example.test", prepared.raw())
            .await
            .unwrap(),
        SentOutcome::ProviderManaged
    );
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.submit(&prepared, &gate).await,
        SubmissionOutcome::DeliveryUnknown
    );
    assert_eq!(writes.load(Ordering::SeqCst), 2);
    prepared.raw.resize(4 * 1024 * 1024 + 1, b' ');
    assert_eq!(
        provider.submit(&prepared, &gate).await,
        SubmissionOutcome::Failed {
            kind: ErrorKind::Unsupported
        }
    );
    assert_eq!(
        gate.0.load(Ordering::SeqCst),
        2,
        "oversized mail must fail before the durable send gate"
    );
    assert_eq!(writes.load(Ordering::SeqCst), 2);
    task.abort();
}
#[tokio::test]
async fn missing_headers_use_bounded_mime_without_losing_repeated_or_encoded_headers() {
    let (provider, task) = fixture(|_, uri, _, _| {
        if uri.path().ends_with("/$value") {
            return "From: sender@example.test\r\nSubject: =?UTF-8?Q?Caf=C3=A9?=\r\nMessage-ID: <fixture@example.test>\r\nReferences: <first@example.test>\r\nReferences: <ancestor@example.test>\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nhello=20world".into_response();
        }
        let mut remote = message("opaque-id");
        remote["internetMessageHeaders"] = serde_json::json!([]);
        json(serde_json::json!({"value":[remote]}))
    }).await;
    *provider.folders.lock().await = Some((
        Instant::now(),
        vec![GraphFolder {
            id: "inbox-id".into(),
            folder: Folder {
                name: "INBOX".into(),
                selectable: true,
                attributes: vec!["Inbox".into()],
            },
        }],
    ));
    let query = Search {
        text: Some("hello world".into()),
        conversation_id: Some("first@example.test".into()),
        subject: Some("café".into()),
        before: Some("30-Oct-2026".into()),
        ..Default::default()
    };
    let page = provider.search("INBOX", &query, None, 10).await.unwrap();
    assert_eq!(page.rows.len(), 1);
    let headers = String::from_utf8_lossy(&page.rows[0].headers);
    assert!(
        headers.contains("References: <first@example.test>\r\nReferences: <ancestor@example.test>")
    );
    assert!(!headers.contains("hello"));
    assert!(page.rows[0].size > 0);
    task.abort();
}
#[tokio::test]
async fn throttling_honors_cooldown_and_authentication_failures_require_reconnection() {
    let (provider, task) = fixture(|_, _, _, _| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "120")],
            "private error detail",
        )
            .into_response()
    })
    .await;
    let reference = provider.reference("INBOX", "opaque-id".into());
    assert_eq!(
        provider.fetch(&reference, 1024).await.unwrap_err().kind,
        ErrorKind::Retryable
    );
    assert_eq!(
        provider.fetch(&reference, 1024).await.unwrap_err().message,
        "Microsoft Graph throttled"
    );
    task.abort();
    let (provider, task) = fixture(|_, _, _, _| StatusCode::FORBIDDEN.into_response()).await;
    assert_eq!(
        provider.fetch(&reference, 1024).await.unwrap_err().kind,
        ErrorKind::Authentication
    );
    task.abort();
}
