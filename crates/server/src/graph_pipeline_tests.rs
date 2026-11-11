use super::*;
use crate::{App, Config};
use axum::{
    body::{Body, Bytes},
    extract::Request,
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use p::MailProvider;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex as StdMutex,
    },
};
use tower::ServiceExt;

struct Token;
#[async_trait::async_trait]
impl p::AccessToken for Token {
    async fn access_token(&self) -> p::Result<String> {
        Ok("fixture-access".into())
    }
}
#[derive(Default)]
struct Fake {
    base: StdMutex<String>,
    messages: StdMutex<HashMap<String, Value>>,
    delta: StdMutex<VecDeque<(u16, Value)>>,
    arrivals: StdMutex<Vec<Value>>,
    batches: AtomicUsize,
    sends: AtomicUsize,
    send_status: StdMutex<VecDeque<u16>>,
    sent: AtomicBool,
    immutable: AtomicBool,
    create_status: StdMutex<Option<u16>>,
    subscription_creates: AtomicUsize,
    subscription_renews: AtomicUsize,
    subscription_deletes: AtomicUsize,
    client_state: StdMutex<String>,
    batch_status: StdMutex<HashMap<String, u16>>,
}
fn response(body: Value) -> Response {
    axum::Json(body).into_response()
}
fn message(id: &str, rfc: &str) -> Value {
    json!({"id":id,"isRead":false,"flag":{"flagStatus":"notFlagged"},"receivedDateTime":"2026-10-29T12:00:00Z",
        "internetMessageHeaders":[{"name":"From","value":"sender@example.test"},{"name":"To","value":"fixture@example.test"},
        {"name":"Subject","value":"Fixture"},{"name":"Message-ID","value":format!("<{rfc}@example.test>")}]})
}
fn delta(items: Value, next: bool) -> Value {
    let mut value = json!({"value":items});
    value[if next {
        "@odata.nextLink"
    } else {
        "@odata.deltaLink"
    }] = json!(format!(
        "me/mailfolders('inbox-id')/messages/delta?{}=fixture",
        if next { "$skiptoken" } else { "$deltatoken" }
    ));
    value
}
impl Fake {
    fn answer(&self, method: axum::http::Method, uri: axum::http::Uri, body: Bytes) -> Response {
        let path = uri.path();
        if path == "/v1.0/me/messages" {
            let mut response = response(json!({"value":[]}));
            if self.immutable.load(Ordering::SeqCst) {
                response.headers_mut().insert(
                    "preference-applied",
                    "IdType=\"ImmutableId\"".parse().unwrap(),
                );
            }
            return response;
        }
        if path == "/v1.0/me/mailFolders/inbox" {
            return response(json!({"id":"inbox-id","displayName":"Inbox","childFolderCount":0}));
        }
        if path == "/v1.0/me/mailFolders" {
            return response(
                json!({"value":[{"id":"inbox-id","displayName":"Inbox","childFolderCount":0}]}),
            );
        }
        if path.ends_with("/messages/delta") {
            let (status, body) = self
                .delta
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected delta request");
            return (
                axum::http::StatusCode::from_u16(status).unwrap(),
                axum::Json(body),
            )
                .into_response();
        }
        if path == "/v1.0/me/mailFolders/inbox-id/messages" {
            return response(json!({"value":self.arrivals.lock().unwrap().clone()}));
        }
        if path == "/v1.0/me/mailFolders/sentitems/messages" {
            return response(
                json!({"value":if self.sent.load(Ordering::SeqCst){vec![json!({"id":"sent-id"})]}else{vec![]}}),
            );
        }
        if path == "/v1.0/me/sendMail" {
            self.sends.fetch_add(1, Ordering::SeqCst);
            let status = self.send_status.lock().unwrap().pop_front().unwrap_or(202);
            return (
                axum::http::StatusCode::from_u16(status).unwrap(),
                [("retry-after", "1")],
                "",
            )
                .into_response();
        }
        if path == "/v1.0/subscriptions" && method == axum::http::Method::POST {
            let number = self.subscription_creates.fetch_add(1, Ordering::SeqCst);
            if let Some(status) = *self.create_status.lock().unwrap() {
                return axum::http::StatusCode::from_u16(status)
                    .unwrap()
                    .into_response();
            }
            let body: Value = serde_json::from_slice(&body).unwrap();
            *self.client_state.lock().unwrap() = body["clientState"].as_str().unwrap().into();
            return response(
                json!({"id":if number==0 {"subscription-id".to_owned()} else {format!("subscription-{number}")},"expirationDateTime":body["expirationDateTime"]}),
            );
        }
        if path.starts_with("/v1.0/subscriptions/") {
            if method == axum::http::Method::DELETE {
                self.subscription_deletes.fetch_add(1, Ordering::SeqCst);
                return axum::http::StatusCode::NO_CONTENT.into_response();
            }
            self.subscription_renews.fetch_add(1, Ordering::SeqCst);
            let body: Value = serde_json::from_slice(&body).unwrap();
            return response(
                json!({"id":path.rsplit('/').next().unwrap(),"expirationDateTime":body["expirationDateTime"]}),
            );
        }
        if path == "/v1.0/$batch" {
            self.batches.fetch_add(1, Ordering::SeqCst);
            let body: Value = serde_json::from_slice(&body).unwrap();
            let responses: Vec<_> = body["requests"]
                .as_array()
                .unwrap()
                .iter()
                .rev()
                .map(|r| {
                    let url = r["url"].as_str().unwrap();
                    let id = url
                        .trim_start_matches("/me/messages/")
                        .split(['?', '/'])
                        .next()
                        .unwrap();
                    let status = self.batch_status.lock().unwrap().get(id).copied();
                    if let Some(status) = status {
                        return json!({"id":r["id"],"status":status,"headers":{"Retry-After":"1"}});
                    }
                    if r["method"] == "GET" {
                        let messages = self.messages.lock().unwrap();
                        match messages.get(id) {
                            Some(message) => json!({"id":r["id"],"status":200,"body":message}),
                            None => json!({"id":r["id"],"status":404}),
                        }
                    } else {
                        json!({"id":r["id"],"status":200,"body":{"id":id}})
                    }
                })
                .collect();
            return response(json!({"responses":responses}));
        }
        if let Some(id) = path.strip_prefix("/v1.0/me/messages/") {
            if self.messages.lock().unwrap().contains_key(id) {
                return response(json!({"id":id}));
            }
        }
        axum::http::StatusCode::NOT_FOUND.into_response()
    }
}
async fn setup(
    immutable: bool,
) -> (
    tempfile::TempDir,
    App,
    AuthorizedAccount,
    Arc<p::GraphProvider>,
    Arc<Fake>,
    tokio::task::JoinHandle<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path(), [7; 32]).await.unwrap();
    let user = store
        .user("https://fixture.invalid", "owner")
        .await
        .unwrap();
    let input:ConnectAccount=serde_json::from_value(json!({"email":"fixture@example.test","display_name":"Fixture","provider":"microsoft",
        "imap_host":"graph.microsoft.com","imap_port":443,"imap_tls":"implicit","smtp_host":"graph.microsoft.com","smtp_port":443,"smtp_tls":"implicit",
        "username":"fixture@example.test","password":"","microsoft_oauth":{"client_id":"fixture","tenant":"common","subject":"fixture-user","refresh_token":"fixture-refresh"}})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let scope = store.authorize(&user, &account.id).await.unwrap();
    let fake = Arc::new(Fake::default());
    fake.immutable.store(immutable, Ordering::SeqCst);
    let state = fake.clone();
    let router = Router::new().fallback(any(move |request: Request| {
        let state = state.clone();
        async move {
            let (parts, body) = request.into_parts();
            let body = axum::body::to_bytes(body, 8 * 1024 * 1024).await.unwrap();
            state.answer(parts.method, parts.uri, body)
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    *fake.base.lock().unwrap() = format!("http://{address}/v1.0/");
    let graph = Arc::new(
        p::GraphProvider::new(account.id, Arc::new(Token), Default::default())
            .with_test_endpoint(url::Url::parse(&format!("http://{address}/v1.0/")).unwrap()),
    );
    let runtime = MailRuntime::default();
    runtime
        .providers
        .lock()
        .await
        .insert(scope.id().into(), graph.clone());
    let app = App {
        store,
        mail: runtime,
        config: Arc::new(Config {
            origin: "https://mail.example.invalid".into(),
            issuer: "https://fixture.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture".into(),
            web_root: directory.path().to_string_lossy().into(),
        }),
    };
    app.mail.probe(&app.store, &scope).await.unwrap();
    (directory, app, scope, graph, fake, task)
}
async fn tick(app: &App, a: &AuthorizedAccount) {
    crate::sync::tick(app, a).await.unwrap();
}
async fn force_delta(app: &App, a: &AuthorizedAccount) {
    let raw = app
        .store
        .provider_document(a, "graph:cursor:INBOX")
        .await
        .unwrap()
        .unwrap();
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    value["at"] = json!(0);
    app.store
        .put_provider_document(a, "graph:cursor:INBOX", &value.to_string())
        .await
        .unwrap();
}
async fn head_due(app: &App, a: &AuthorizedAccount) {
    let raw = app
        .store
        .provider_document(a, "graph:arrivals")
        .await
        .unwrap()
        .unwrap();
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    value["at"] = json!(0);
    app.store
        .put_provider_document(a, "graph:arrivals", &value.to_string())
        .await
        .unwrap();
}
#[tokio::test]
async fn graph_delta_import_restart_arrivals_and_removals_preserve_unmentioned_mail() {
    let (_d, mut app, mut a, _graph, fake, task) = setup(true).await;
    for id in ["a", "b", "new"] {
        fake.messages
            .lock()
            .unwrap()
            .insert(id.into(), message(id, id));
    }
    fake.delta.lock().unwrap().extend([(200,delta(json!([{"id":"a"},{"id":"b"}]),true)),
        (200,delta(json!([{"id":"a","@removed":{}},{"id":"b","isRead":true,"flag":{"flagStatus":"flagged"}}]),false)),
        (200,delta(json!([{"id":"b","isRead":false,"flag":{"flagStatus":"notFlagged"}}]),false))]);
    tick(&app, &a).await; // Independent arrival bootstrap.
    tick(&app, &a).await; // First import page.
    let b = app
        .store
        .known_provider_message(&a, "graph:b")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b.folder_ids, vec!["INBOX"]);
    fake.arrivals.lock().unwrap().push(message("new", "new"));
    head_due(&app, &a).await;
    tick(&app, &a).await; // New mail is visible before history completes.
    let new = app
        .store
        .known_provider_message(&a, "graph:new")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new.folder_ids, vec!["INBOX"]);
    let user = a.user_id().to_owned();
    let id = a.id().to_owned();
    app.store = Store::open(_d.path(), [7; 32]).await.unwrap();
    a = app.store.authorize(&user, &id).await.unwrap();
    app.mail = MailRuntime::default();
    let restarted = Arc::new(
        p::GraphProvider::new(id.clone(), Arc::new(Token), Default::default())
            .with_test_endpoint(url::Url::parse(&fake.base.lock().unwrap()).unwrap()),
    );
    app.mail.providers.lock().await.insert(id, restarted);
    app.mail.probe(&app.store, &a).await.unwrap();
    tick(&app, &a).await; // Resume the persisted cursor after reopening the store/provider.
    assert!(app
        .store
        .known_provider_message(&a, "graph:a")
        .await
        .unwrap()
        .unwrap()
        .folder_ids
        .is_empty());
    let b = app.store.message(&a, &b.id).await.unwrap();
    assert!(!b.unread && b.starred);
    assert_eq!(
        app.store.message(&a, &new.id).await.unwrap().folder_ids,
        vec!["INBOX"]
    );
    force_delta(&app, &a).await;
    tick(&app, &a).await;
    let b = app.store.message(&a, &b.id).await.unwrap();
    assert!(b.unread && !b.starred);
    assert_eq!(
        app.store.message(&a, &new.id).await.unwrap().folder_ids,
        vec!["INBOX"],
        "incremental delta must not prune unmentioned messages"
    );
    assert_eq!(
        fake.batches.load(Ordering::SeqCst),
        1,
        "known mutable deltas must not refetch headers or bodies"
    );
    task.abort();
}
#[tokio::test]
async fn graph_expired_cursor_restarts_without_pruning_or_losing_coverage() {
    let (_d, app, a, _graph, fake, task) = setup(true).await;
    fake.messages
        .lock()
        .unwrap()
        .insert("a".into(), message("a", "a"));
    fake.delta.lock().unwrap().extend([
        (200, delta(json!([{"id":"a"}]), true)),
        (410, json!({})),
        (200, delta(json!([{"id":"a"}]), false)),
    ]);
    tick(&app, &a).await;
    tick(&app, &a).await;
    let before = app
        .store
        .known_provider_message(&a, "graph:a")
        .await
        .unwrap()
        .unwrap();
    tick(&app, &a).await;
    assert!(app
        .store
        .provider_document(&a, "graph:cursor:INBOX")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        app.store.message(&a, &before.id).await.unwrap().folder_ids,
        vec!["INBOX"]
    );
    tick(&app, &a).await;
    assert_eq!(
        app.store.message(&a, &before.id).await.unwrap().folder_ids,
        vec!["INBOX"]
    );
    task.abort();
}
#[tokio::test]
async fn graph_external_moves_relink_only_a_unique_missing_locator_and_never_merge_live_copies() {
    let (_d, app, a, graph, fake, task) = setup(false).await;
    for id in ["old", "copy", "moved"] {
        fake.messages
            .lock()
            .unwrap()
            .insert(id.into(), message(id, "same-rfc"));
    }
    let first = graph.search("INBOX", &p::Search::default(), None, 10).await;
    assert!(first.is_ok());
    // Ingest protocol metadata through delta so all account-local evidence is real.
    fake.delta.lock().unwrap().extend([
        (200, delta(json!([{"id":"old"}]), false)),
        (200, delta(json!([{"id":"copy"}]), false)),
        (
            200,
            delta(json!([{"id":"moved"},{"id":"old","@removed":{}}]), false),
        ),
    ]);
    tick(&app, &a).await;
    tick(&app, &a).await;
    let old = app
        .store
        .known_provider_message(&a, "graph:old")
        .await
        .unwrap()
        .unwrap();
    force_delta(&app, &a).await;
    tick(&app, &a).await;
    let copy = app
        .store
        .known_provider_message(&a, "graph:copy")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        copy.id, old.id,
        "a matching live copy must keep its own identity"
    );
    // Remove the copy's header evidence to keep the remaining old candidate unique.
    app.store
        .put_provider_document(&a, &format!("thread-headers:{}", copy.id), "{}")
        .await
        .unwrap();
    fake.messages.lock().unwrap().remove("old");
    force_delta(&app, &a).await;
    tick(&app, &a).await;
    let moved = app
        .store
        .known_provider_message(&a, "graph:moved")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.id, old.id);
    assert_eq!(
        moved.folder_ids,
        vec!["INBOX"],
        "old locator tombstones cannot remove its newly relinked membership"
    );
    task.abort();
}
#[tokio::test]
async fn graph_removals_clear_old_memberships_without_erasing_a_new_destination() {
    let (_d, app, a, _graph, fake, task) = setup(true).await;
    fake.messages
        .lock()
        .unwrap()
        .insert("a".into(), message("a", "a"));
    fake.delta
        .lock()
        .unwrap()
        .push_back((200, delta(json!([{"id":"a"}]), false)));
    tick(&app, &a).await;
    tick(&app, &a).await;
    let mut moved = app
        .store
        .known_provider_message(&a, "graph:a")
        .await
        .unwrap()
        .unwrap();
    moved.folder_ids = vec!["Archive".into()];
    let reference = p::MessageRef {
        account_id: a.id().into(),
        folder: "Archive".into(),
        graph_id: Some("a".into()),
        uid: 1,
        uidvalidity: 1,
    };
    app.store
        .ingest_messages(
            &a,
            vec![moved.clone()],
            &[(
                format!("reference:{}", moved.id),
                serde_json::to_string(&reference).unwrap(),
            )],
        )
        .await
        .unwrap();
    let checkpoint = app
        .store
        .provider_document(&a, "graph:cursor:INBOX")
        .await
        .unwrap()
        .unwrap();
    app.store
        .commit_graph_page(&a, "INBOX", &[], &["a".into()], &checkpoint, false, true)
        .await
        .unwrap();
    assert_eq!(
        app.store.message(&a, &moved.id).await.unwrap().folder_ids,
        vec!["Archive"]
    );
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        _d.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM folder_memberships WHERE folder='INBOX' AND message_id=?",
    )
    .bind(&moved.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        count, 0,
        "a completed source removal must not leave stale membership counts"
    );
    task.abort();
}
async fn submission(app: &App, a: &AuthorizedAccount) -> Submission {
    let draft:Draft=serde_json::from_value(json!({"id":"draft","account_id":a.id(),"revision":0,"to":[{"email":"recipient@example.test","name":null}],"cc":[],"bcc":[],"subject":"Fixture","text":"Hello","html":"<p>Hello</p>","in_reply_to":null,"references":[],"attachments":[],"saved_at":""})).unwrap();
    let draft = app.store.save_draft(a, draft).await.unwrap();
    app.store
        .submit(
            a,
            &SubmitRequest {
                operation_id: "send-once".into(),
                draft_id: draft.id,
                expected_revision: draft.revision,
                due_at: None,
                timezone: "UTC".into(),
            },
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn graph_send_deferral_is_durable_but_unknown_delivery_only_recovers_with_positive_sent_evidence(
) {
    let (_d, app, a, _graph, fake, task) = setup(true).await;
    fake.send_status.lock().unwrap().extend([503, 502]);
    let submission = submission(&app, &a).await;
    crate::executor::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store
            .submission(&a, &submission.id)
            .await
            .unwrap()
            .state,
        SubmissionState::Scheduled
    );
    assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
    // The retry retains its snapshot and MIME. Simulate the durable delay expiring.
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        _d.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE submissions SET due_at='2000-01-01T00:00:00.000Z' WHERE id=?")
        .bind(&submission.id)
        .execute(&db)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    crate::executor::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store
            .submission(&a, &submission.id)
            .await
            .unwrap()
            .state,
        SubmissionState::DeliveryUnknown
    );
    crate::executor::tick(&app, &a).await.unwrap();
    assert_eq!(fake.sends.load(Ordering::SeqCst), 2);
    fake.sent.store(true, Ordering::SeqCst);
    app.store
        .put_provider_document(&a, &format!("send:lookup:{}", submission.id), "0")
        .await
        .unwrap();
    crate::executor::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store
            .submission(&a, &submission.id)
            .await
            .unwrap()
            .state,
        SubmissionState::Accepted
    );
    assert_eq!(
        fake.sends.load(Ordering::SeqCst),
        2,
        "a lost acknowledgement must never cause an automatic resend"
    );
    assert!(app
        .store
        .defer_submission(&a, &submission.id, SubmissionState::DeliveryUnknown, 1)
        .await
        .is_err());
    task.abort();
}
#[tokio::test]
async fn graph_bulk_mutations_keep_per_operation_outcomes_and_retry_only_refused_writes() {
    let (_d, app, a, _graph, fake, task) = setup(true).await;
    for id in ["a", "b", "c"] {
        fake.messages
            .lock()
            .unwrap()
            .insert(id.into(), message(id, id));
    }
    fake.delta
        .lock()
        .unwrap()
        .push_back((200, delta(json!([{"id":"a"},{"id":"b"},{"id":"c"}]), false)));
    tick(&app, &a).await;
    tick(&app, &a).await;
    for id in ["a", "b", "c"] {
        let message = app
            .store
            .known_provider_message(&a, &format!("graph:{id}"))
            .await
            .unwrap()
            .unwrap();
        app.store
            .operation(
                &a,
                Operation {
                    id: format!("bulk-{id}"),
                    account_id: a.id().into(),
                    target_id: message.id,
                    expected_revision: message.revision,
                    change: Change::Read { value: true },
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
    }
    fake.batch_status
        .lock()
        .unwrap()
        .extend([("b".into(), 429), ("c".into(), 403)]);
    let before = fake.batches.load(Ordering::SeqCst);
    crate::mutations::drain(&app, &a).await.unwrap();
    assert_eq!(fake.batches.load(Ordering::SeqCst), before + 1);
    assert_eq!(
        app.store
            .operation_status(&a, "bulk-a")
            .await
            .unwrap()
            .status,
        OperationStatus::Applied
    );
    assert_eq!(
        app.store
            .operation_status(&a, "bulk-b")
            .await
            .unwrap()
            .status,
        OperationStatus::Pending
    );
    assert_eq!(
        app.store
            .operation_status(&a, "bulk-c")
            .await
            .unwrap()
            .status,
        OperationStatus::Failed
    );
    task.abort();
}
#[tokio::test]
async fn graph_webhook_validates_synchronously_checks_secrets_and_subscriptions_renew_and_fall_back(
) {
    let (_d, mut app, a, graph, fake, task) = setup(true).await;
    Arc::get_mut(&mut app.config).unwrap().origin = "https://mail.example.com".into();
    crate::graph_push::maintain(&app, &a, &graph).await.unwrap();
    assert_eq!(fake.subscription_creates.load(Ordering::SeqCst), 1);
    let secret = fake.client_state.lock().unwrap().clone();
    let router = crate::router(app.clone());
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/microsoft/notifications?validationToken=encoded%2Btoken")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
        "encoded+token"
    );
    for (secret, expected) in [("wrong".to_owned(), 0), (secret.clone(), 1)] {
        let body = json!({"value":[{"subscriptionId":"subscription-id","clientState":secret}]})
            .to_string();
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/microsoft/notifications")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(
            crate::graph_push::wake_version(&app.store, &a)
                .await
                .unwrap(),
            expected
        );
    }
    sqlx::query("UPDATE microsoft_subscriptions SET expires_at=? WHERE account_id=?")
        .bind((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339())
        .bind(a.id())
        .execute(app.store.control())
        .await
        .unwrap();
    crate::graph_push::maintain(&app, &a, &graph).await.unwrap();
    assert_eq!(fake.subscription_renews.load(Ordering::SeqCst), 1);
    assert_eq!(fake.client_state.lock().unwrap().as_str(), secret);
    for (event, creates, renews) in [
        ("reauthorizationRequired", 1, 2),
        ("subscriptionRemoved", 2, 2),
    ] {
        let body=json!({"value":[{"subscriptionId":"subscription-id","clientState":secret,"lifecycleEvent":event}]}).to_string();
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/microsoft/notifications")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        crate::graph_push::maintain(&app, &a, &graph).await.unwrap();
        assert_eq!(fake.subscription_creates.load(Ordering::SeqCst), creates);
        assert_eq!(fake.subscription_renews.load(Ordering::SeqCst), renews);
    }
    let wake = crate::graph_push::wake_version(&app.store, &a)
        .await
        .unwrap();
    let obsolete =
        json!({"value":[{"subscriptionId":"subscription-id","clientState":secret}]}).to_string();
    router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/microsoft/notifications")
                .body(Body::from(obsolete))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        crate::graph_push::wake_version(&app.store, &a)
            .await
            .unwrap(),
        wake,
        "obsolete subscription ids cannot wake another registration"
    );
    let oversized = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/microsoft/notifications")
                .body(Body::from(vec![b' '; 65537]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let before_delete = fake.subscription_deletes.load(Ordering::SeqCst);
    assert!(app
        .mail
        .remove_account(&app.store, &a, "wrong@example.test")
        .await
        .is_err());
    assert_eq!(
        fake.subscription_deletes.load(Ordering::SeqCst),
        before_delete
    );
    app.mail
        .remove_account(&app.store, &a, "fixture@example.test")
        .await
        .unwrap();
    assert_eq!(
        fake.subscription_deletes.load(Ordering::SeqCst),
        before_delete + 1
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM microsoft_subscriptions")
        .fetch_one(app.store.control())
        .await
        .unwrap();
    assert_eq!(count, 0);
    task.abort();

    let (_d, mut app, a, graph, fake, task) = setup(true).await;
    Arc::get_mut(&mut app.config).unwrap().origin = "https://mail.example.com".into();
    *fake.create_status.lock().unwrap() = Some(403);
    crate::graph_push::maintain(&app, &a, &graph).await.unwrap();
    crate::graph_push::maintain(&app, &a, &graph).await.unwrap();
    assert_eq!(
        fake.subscription_creates.load(Ordering::SeqCst),
        1,
        "registration failures must back off while polling remains available"
    );
    fake.delta
        .lock()
        .unwrap()
        .push_back((200, delta(json!([]), false)));
    tick(&app, &a).await;
    tick(&app, &a).await;
    assert!(app
        .store
        .provider_document(&a, "graph:cursor:INBOX")
        .await
        .unwrap()
        .is_some());
    task.abort();
}
