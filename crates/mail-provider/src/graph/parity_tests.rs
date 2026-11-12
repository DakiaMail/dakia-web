use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{json as value, Value};
use std::collections::HashMap;

#[tokio::test]
async fn graph_download_limits_cover_declared_and_streamed_bodies_without_replaying_writes() {
    let (provider, task) = fixture(|_, uri, _, _| {
        if uri.path().ends_with("/declared") {
            return vec![b'a'; 17].into_response();
        }
        if uri.path().ends_with("/streamed") {
            return axum::response::Response::new(axum::body::Body::from_stream(
                futures_util::stream::iter([
                    Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![b'a'; 12])),
                    Ok(axum::body::Bytes::from(vec![b'b'; 12])),
                ]),
            ));
        }
        vec![b'a'; 24 * 1024 * 1024 + 1].into_response()
    })
    .await;
    for path in ["me/declared", "me/streamed"] {
        let response = provider
            .request(Method::GET, path)
            .await
            .unwrap()
            .send()
            .await
            .unwrap();
        assert_eq!(
            GraphProvider::bounded(response, 16).await.unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }
    let change = (
        provider.reference("INBOX", "one".into()),
        Mutation::Seen(true),
    );
    assert_eq!(
        provider.mutate_batch(&[change]).await.unwrap_err().kind,
        ErrorKind::Uncertain
    );
    task.abort();
}
async fn folder(provider: &GraphProvider) {
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
}
struct Known(bool);
#[async_trait]
impl MetadataLookup for Known {
    async fn identity_known(&self, _: &Identity) -> Result<bool> {
        Ok(self.0)
    }
    async fn reference_known(&self, _: &MessageRef) -> Result<bool> {
        Ok(self.0)
    }
}
#[tokio::test]
async fn delta_uses_opaque_odata_cursors_batches_new_rows_and_returns_only_actual_changes() {
    let requests = Arc::new(AtomicUsize::new(0));
    let calls = requests.clone();
    let (provider,task) = fixture(move |method,uri,headers,body| {
        calls.fetch_add(1,Ordering::SeqCst);
        if uri.path()=="/v1.0/$batch" {
            assert_eq!(method,Method::POST);
            let body: Value=serde_json::from_slice(&body).unwrap();
            let requests=body["requests"].as_array().unwrap();
            assert_eq!(requests.len(),2);
            assert!(requests.iter().all(|r| r["headers"]["Prefer"]=="IdType=\"ImmutableId\""));
            return json(value!({"responses":[
                {"id":"1","status":200,"body":message("b")},
                {"id":"0","status":200,"body":message("a")}
            ]}));
        }
        assert!(headers["prefer"].to_str().unwrap().contains("odata.maxpagesize=200"));
        if uri.path().contains("mailfolders('") {
            assert!(uri.query().unwrap().contains("$skiptoken=opaque"));
            return json(value!({"value":[{"id":"a","@removed":{"reason":"deleted"}},{"id":"b","isRead":true,"flag":{"flagStatus":"notFlagged"}}],
                "@odata.deltaLink":"me/mailfolders('inbox-id')/messages/delta?$deltatoken=opaque"}));
        }
        assert!(uri.query().unwrap().contains("receivedDateTime%20desc"));
        json(value!({"value":[{"id":"a"},{"id":"b"}],"@odata.nextLink":"me/mailfolders('inbox-id')/messages/delta?$skiptoken=opaque"}))
    }).await;
    folder(&provider).await;
    let first = provider.delta("INBOX", None, &Known(false)).await.unwrap();
    assert_eq!(first.messages.len(), 2);
    assert_eq!(first.messages[0].reference.graph_id.as_deref(), Some("a"));
    assert!(first.cursor.initial && first.cursor.next);
    let second = provider
        .delta("INBOX", Some(&first.cursor), &Known(true))
        .await
        .unwrap();
    assert!(second.messages.is_empty());
    assert_eq!(second.removed, vec!["a"]);
    assert_eq!(second.updates.len(), 1);
    assert!(second.updates[0].seen);
    assert!(!second.updates[0].starred);
    assert!(!second.cursor.initial && !second.cursor.next);
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    task.abort();
}
#[tokio::test]
async fn expired_and_replaced_folder_cursors_are_stale_but_foreign_urls_never_receive_tokens() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (provider, task) = fixture(move |_, _, _, _| {
        count.fetch_add(1, Ordering::SeqCst);
        StatusCode::GONE.into_response()
    })
    .await;
    folder(&provider).await;
    let mut cursor = GraphCursor {
        folder_id: "old-folder".into(),
        url: "me/mailFolders/old-folder/messages/delta?$skiptoken=old".into(),
        initial: true,
        next: true,
    };
    assert_eq!(
        provider
            .delta("INBOX", Some(&cursor), &Known(false))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::StaleReference
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    cursor.folder_id = "inbox-id".into();
    cursor.url = "https://attacker.test/v1.0/me/mailFolders/inbox-id/messages/delta".into();
    assert_eq!(
        provider
            .delta("INBOX", Some(&cursor), &Known(false))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        provider
            .delta("INBOX", None, &Known(false))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::StaleReference
    );
    task.abort();
    let (provider, task) = fixture(|_, _, _, _| {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(value!({"error":{"code":"SyncStateNotFound"}})),
        )
            .into_response()
    })
    .await;
    folder(&provider).await;
    assert_eq!(
        provider
            .delta("INBOX", None, &Known(false))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::StaleReference
    );
    task.abort();
}
#[tokio::test]
async fn delta_keeps_last_observation_and_refuses_partial_batch_failures_without_a_checkpoint() {
    let (provider,task)=fixture(|_,uri,_,_| {
        if uri.path().ends_with("$batch") {
            return json(value!({"responses":[{"id":"0","status":429,"headers":{"Retry-After":"7"}}]}));
        }
        json(value!({"value":[{"id":"a","@removed":{}},{"id":"a","isRead":false,"flag":{"flagStatus":"flagged"}}],
            "@odata.deltaLink":"me/mailFolders/inbox-id/messages/delta?$deltatoken=next"}))
    }).await;
    folder(&provider).await;
    assert_eq!(
        provider
            .delta("INBOX", None, &Known(false))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::Retryable
    );
    assert!(provider.cooldown.lock().await.is_some());
    task.abort();
}
#[tokio::test]
async fn mutation_batches_isolate_throttled_refused_and_missing_subresponses() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (provider,task)=fixture(move |_,uri,_,body| {
        count.fetch_add(1,Ordering::SeqCst);assert_eq!(uri.path(),"/v1.0/$batch");
        let body:Value=serde_json::from_slice(&body).unwrap();assert_eq!(body["requests"].as_array().unwrap().len(),4);
        json(value!({"responses":[{"id":"2","status":403},{"id":"1","status":204},{"id":"0","status":429,"headers":{"retry-after":"9"}}]}))
    }).await;
    let changes: Vec<_> = (0..4)
        .map(|i| {
            (
                provider.reference("INBOX", format!("id-{i}")),
                Mutation::Seen(true),
            )
        })
        .collect();
    let results = provider.mutate_batch(&changes).await.unwrap();
    assert_eq!(results[0].as_ref().unwrap_err().kind, ErrorKind::Retryable);
    assert!(results[1].is_ok());
    assert_eq!(
        results[2].as_ref().unwrap_err().kind,
        ErrorKind::Authentication
    );
    assert_eq!(results[3].as_ref().unwrap_err().kind, ErrorKind::Uncertain);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    task.abort();
}
#[tokio::test]
async fn body_batches_split_at_twenty_and_preserve_order_with_base64url_mime() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (provider,task)=fixture(move |_,_,_,body| {
        count.fetch_add(1,Ordering::SeqCst);
        let body:Value=serde_json::from_slice(&body).unwrap();
        let requests=body["requests"].as_array().unwrap(); assert!(requests.len()<=20);
        let responses:Vec<_>=requests.iter().rev().map(|r| value!({"id":r["id"],"status":200,"body":URL_SAFE_NO_PAD.encode(r["url"].as_str().unwrap().as_bytes())})).collect();
        json(value!({"responses":responses}))
    }).await;
    let references: Vec<_> = (0..45)
        .map(|i| provider.reference("INBOX", format!("opaque-{i}")))
        .collect();
    let bodies = provider.fetch_batch(&references, 1024).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    for (i, body) in bodies.iter().enumerate() {
        assert!(String::from_utf8_lossy(body).contains(&format!("opaque-{i}/$value")));
    }
    task.abort();
}
fn prepared() -> PreparedMessage {
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
    mime::assemble(
        &config,
        mime::Compose {
            to: vec!["recipient@example.test".into()],
            cc: vec![],
            bcc: vec![],
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
    .unwrap()
}
#[tokio::test]
async fn sending_classifies_definitive_refusals_separately_from_lost_acknowledgements() {
    for (status, deferred, unknown) in [
        (202, false, false),
        (429, true, false),
        (503, true, false),
        (500, true, false),
        (502, false, true),
        (504, false, true),
        (403, false, false),
    ] {
        let (provider, task) = fixture(move |_, _, _, _| {
            (
                StatusCode::from_u16(status).unwrap(),
                [("retry-after", "7")],
                "",
            )
                .into_response()
        })
        .await;
        let gate = Gate(AtomicUsize::new(0));
        let result = provider.submit(&prepared(), &gate).await;
        assert_eq!(
            matches!(result, SubmissionOutcome::Deferred { .. }),
            deferred,
            "status {status}"
        );
        assert_eq!(
            result == SubmissionOutcome::DeliveryUnknown,
            unknown,
            "status {status}"
        );
        assert_eq!(gate.0.load(Ordering::SeqCst), 1);
        task.abort();
    }
}
#[tokio::test]
async fn four_mib_mime_parity_and_sent_lookup_never_resubmit() {
    let (provider, task) = fixture(|method, uri, _, body| {
        if method == Method::GET {
            let query = Url::parse(&format!("https://fixture.test{uri}"))
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect::<HashMap<_, _>>();
            assert_eq!(
                query["$filter"],
                "internetMessageId eq '<o''neil@example.test>'"
            );
            return json(value!({"value":[{"id":"sent-id"}]}));
        }
        assert!(STANDARD.decode(body).unwrap().len() > 3 * 1024 * 1024);
        StatusCode::ACCEPTED.into_response()
    })
    .await;
    let mut mime = prepared();
    mime.raw.resize(4 * 1024 * 1024 - 1, b' ');
    assert!(matches!(
        provider.submit(&mime, &Gate(AtomicUsize::new(0))).await,
        SubmissionOutcome::Accepted { .. }
    ));
    assert!(provider
        .has_sent_message("<o'neil@example.test>")
        .await
        .unwrap());
    task.abort();
}
#[tokio::test]
async fn subscriptions_create_renew_and_delete_without_mail_writes() {
    let (provider, task) = fixture(|method, uri, _, body| {
        if method == Method::DELETE {
            return StatusCode::NO_CONTENT.into_response();
        }
        let body: Value = serde_json::from_slice(&body).unwrap();
        if method == Method::POST {
            assert_eq!(uri.path(), "/v1.0/subscriptions");
            assert_eq!(body["resource"], "me/messages");
            assert_eq!(body["clientState"], "fixture-secret");
            assert_eq!(body["notificationUrl"], body["lifecycleNotificationUrl"]);
        } else {
            assert_eq!(method, Method::PATCH);
            assert_eq!(uri.path(), "/v1.0/subscriptions/subscription-id");
        }
        json(value!({"id":"subscription-id","expirationDateTime":body["expirationDateTime"]}))
    })
    .await;
    let created = provider
        .subscription(
            "https://mail.test/api/microsoft/notifications",
            "fixture-secret",
            None,
        )
        .await
        .unwrap();
    provider
        .subscription(
            "https://mail.test/api/microsoft/notifications",
            "not-sent-on-renewal",
            Some(&created.id),
        )
        .await
        .unwrap();
    provider.unsubscribe(&created.id).await.unwrap();
    task.abort();
}
