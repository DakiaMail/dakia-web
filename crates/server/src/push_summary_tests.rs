use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use dakia_domain::*;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn newsletter_delivery_gate_reaches_real_worker_only_for_eligible_mail() {
    use std::{
        io::Write,
        process::{Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
    };
    let dir = tempfile::tempdir().unwrap();
    let store = dakia_storage::Store::open(dir.path(), [9; 32])
        .await
        .unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner@example.invalid","password":"synthetic-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let foreign = store.user("fixture", "other").await.unwrap();
    for (owner, id) in [(&user, "one"), (&user, "two"), (&foreign, "foreign")] {
        store.subscribe(owner, id, &serde_json::json!({"id":id,"user_id":owner,"endpoint":"https://fcm.googleapis.com/disposable"}).to_string()).await.unwrap();
    }
    store
        .record_incremental(&a, "INBOX", &[], "bootstrap")
        .await
        .unwrap();
    let app = App {
        store,
        config: Arc::new(crate::Config {
            origin: "https://dakia.example.invalid".into(),
            issuer: "https://id.example.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "synthetic".into(),
            web_root: dir.path().to_string_lossy().into(),
        }),
        mail: crate::mail::MailRuntime::default(),
    };
    let calls = AtomicUsize::new(0);
    let send = |job: serde_json::Value| {
        calls.fetch_add(1, Ordering::SeqCst);
        async move {
            let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../workers/push/test/storage-transport.mjs");
            let mut child = Command::new("node")
                .arg(script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&serde_json::to_vec(&job).unwrap())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["synthetic_transport_calls"], 1);
            assert_eq!(result["external_requests"], 0);
            Ok(result)
        }
    };
    for (id, category) in [
        ("newsletter", Some("newsletters")),
        ("pending", None),
        ("reclassified", Some("people")),
        ("person", Some("people")),
        ("transaction", Some("transactions")),
        ("routine", Some("transactions")),
    ] {
        let mut m: Message = serde_json::from_value(serde_json::json!({"id":id,"account_id":a.id(),"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"Synthetic","preview":"","received_at":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339(),"unread":true,"starred":false,"category":category,"category_status":if category.is_some(){"ready"}else{"pending"},"revision":0})).unwrap();
        m.classification = Some(dakia_domain::Classification {
            kind: if matches!(category, Some("newsletters" | "transactions")) {
                InboxKind::Automated
            } else {
                InboxKind::Personal
            },
            status: if category.is_some() {
                CategoryStatus::Ready
            } else {
                CategoryStatus::Pending
            },
            attention: if id == "transaction" {
                vec![AttentionReason {
                    label: "Payment failed".into(),
                    expires_at: None,
                }]
            } else {
                vec![]
            },
            ..Default::default()
        });
        m = app.store.upsert_message(&a, m).await.unwrap();
        for _ in 0..2 {
            app.store
                .record_incremental(&a, "INBOX", std::slice::from_ref(&m), id)
                .await
                .unwrap();
        }
        // Give admitted fixture mail a current receipt time before delivery.
        m.received_at = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        app.store.upsert_message(&a, m.clone()).await.unwrap();
        if id == "pending" {
            assert!(app.store.next_push(&a).await.unwrap().is_none());
            let mut model = app
                .store
                .queue_model(&a, id, JobKind::Classify, "fixture", None)
                .await
                .unwrap();
            model.status = JobStatus::Succeeded;
            app.store
                .finish_model(&a, model, Some("newsletters"), None)
                .await
                .unwrap();
        }
        if id == "reclassified" {
            let job = app.store.next_push(&a).await.unwrap().unwrap();
            let model = app
                .store
                .queue_model(&a, id, JobKind::Classify, "fixture", None)
                .await
                .unwrap();
            app.store
                .finish_classification(
                    &a,
                    ModelJob {
                        status: JobStatus::Succeeded,
                        ..model
                    },
                    Some(&Classification {
                        kind: InboxKind::Automated,
                        status: CategoryStatus::Ready,
                        ..Default::default()
                    }),
                    None,
                )
                .await
                .unwrap();
            deliver(&app, &a, job, &send).await.unwrap();
        }
        while let Some(job) = app.store.next_push(&a).await.unwrap() {
            deliver(&app, &a, job, &send).await.unwrap();
        }
        if matches!(id, "newsletter" | "pending" | "reclassified" | "routine") {
            assert_eq!(
                calls.load(Ordering::SeqCst),
                if id == "routine" { 4 } else { 0 }
            );
        }
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "two eligible messages to two owner subscriptions, no foreign subscription"
    );
    app.store.recover_push(&a).await.unwrap();
    assert!(app.store.next_push(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn notification_summary_is_authorized_bounded_local_and_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = dakia_storage::Store::open(dir.path(), [9; 32])
        .await
        .unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let other = store.user("fixture", "other").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner@example.invalid","password":"synthetic-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let b = store.connect(&user, &input).await.unwrap();
    let foreign = store.connect(&other, &input).await.unwrap();
    let message = store
        .upsert_message(
            &a,
            Message {
                id: "same-id".into(),
                account_id: account.id.clone(),
                thread_id: "thread".into(),
                provider_id: "opaque".into(),
                folder_ids: vec!["INBOX".into()],
                sender: Address {
                    email: "sender@example.invalid".into(),
                    name: Some(" ".into()),
                },
                to: vec![],
                subject: "😀".repeat(500),
                preview: "Preview ".repeat(100),
                received_at: "2026-10-30T00:00:00Z".into(),
                unread: true,
                starred: false,
                category: None,
                category_status: CategoryStatus::Pending,
                classification: None,
                revision: 0,
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(crate::hash("notification-fixture"))
        .bind(&user)
        .bind("fixture")
        .bind(4102444800_i64)
        .execute(store.control())
        .await
        .unwrap();
    let app = App {
        store,
        config: Arc::new(crate::Config {
            origin: "https://dakia.example.invalid".into(),
            issuer: "https://id.example.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "synthetic".into(),
            web_root: dir.path().to_string_lossy().into(),
        }),
        mail: crate::mail::MailRuntime::default(), // No provider installed: summary must not hydrate.
    };
    let route = crate::router(app.clone());
    for (account, id, authenticated, status) in [
        (&account.id, "same-id", true, StatusCode::OK),
        (&account.id, "missing", true, StatusCode::NOT_FOUND),
        (&b.id, "same-id", true, StatusCode::NOT_FOUND),
        (&foreign.id, "same-id", true, StatusCode::NOT_FOUND),
        (&account.id, "same-id", false, StatusCode::UNAUTHORIZED),
    ] {
        let mut request = Request::builder().uri(format!(
            "/api/accounts/{account}/messages/{id}/notification"
        ));
        if authenticated {
            request = request.header("cookie", "__Host-dakia=notification-fixture");
        }
        let response = route
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
        if status == StatusCode::OK {
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 4);
            assert_eq!(value["account"], "owner@example.invalid");
            assert_eq!(value["sender"], "sender@example.invalid");
            assert_eq!(value["subject"].as_str().unwrap().chars().count(), 160);
            assert_eq!(value["preview"].as_str().unwrap().chars().count(), 240);
        }
    }
    assert_eq!(app.store.message(&a, "same-id").await.unwrap(), message);
}
