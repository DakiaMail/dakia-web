use super::*;
#[test]
fn received_text_attachments_are_files_not_charset_converted_strings() {
    for (encoding, body) in [
        ("base64", "Y2Fm6Qo="),
        ("quoted-printable", "caf=E9=0A"),
        ("7bit", "caf\u{00e9}\n"),
    ] {
        let raw = format!("Content-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Disposition: attachment; filename=test.txt\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n{body}\r\n--x--\r\n");
        let parsed = mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .unwrap();
        let expected = if encoding == "7bit" {
            "caf\u{00e9}\n".as_bytes()
        } else {
            b"caf\xe9\n"
        };
        assert_eq!(
            attachment_bytes(raw.as_bytes(), parsed.attachments().next().unwrap()).unwrap(),
            expected
        );
    }
}
#[test]
fn gmail_system_labels_resolve_to_selectable_mailboxes() {
    let probe = p::Probe {
        capabilities: vec![],
        full_search_coverage: false,
        folders: vec![p::Folder {
            name: "[Gmail]/Sent Mail".into(),
            selectable: true,
            attributes: vec!["Sent".into()],
        }],
    };
    assert_eq!(
        mailbox_memberships(
            "[Gmail]/All Mail",
            vec![
                "\\\\Inbox".into(),
                "\\\\Sent".into(),
                "Project label".into()
            ],
            Some(&probe)
        ),
        vec![
            "INBOX",
            "Project label",
            "[Gmail]/All Mail",
            "[Gmail]/Sent Mail"
        ]
    );
    assert_eq!(
        mailbox_memberships("INBOX", vec!["\\Inbox".into()], None),
        vec!["INBOX"]
    );
    assert_eq!(
        mailbox_memberships("Archive", vec![], None),
        vec!["Archive"]
    );
    assert_eq!(
        mailbox_memberships("INBOX", vec!["\\\\Custom".into()], None),
        vec!["INBOX", "\\\\Custom"]
    );
}
#[path = "inbox_tests.rs"]
mod inbox_tests;
#[path = "search_integration_tests.rs"]
mod search_integration_tests;
#[test]
fn gmail_all_mail_membership_does_not_oscillate_across_scans_or_survive_trash() {
    let probe = p::Probe {
        capabilities: vec![],
        full_search_coverage: false,
        folders: [
            ("[Gmail]/All Mail", "All"),
            ("[Gmail]/Trash", "Trash"),
            ("[Gmail]/Spam", "Junk"),
        ]
        .into_iter()
        .map(|(name, role)| p::Folder {
            name: name.into(),
            selectable: true,
            attributes: vec![role.into()],
        })
        .collect(),
    };
    let previous = vec![
        "INBOX".into(),
        "[Gmail]/All Mail".into(),
        "Old custom label".into(),
    ];
    let mut inbox = mailbox_memberships("INBOX", vec!["\\Inbox".into()], Some(&probe));
    retain_implicit_all_mail(&mut inbox, &previous, &probe);
    let all = mailbox_memberships("[Gmail]/All Mail", vec!["\\Inbox".into()], Some(&probe));
    assert_eq!(inbox, all);
    assert!(!inbox.contains(&"Old custom label".into()));
    for (folder, label) in [("[Gmail]/Trash", "\\Trash"), ("[Gmail]/Spam", "\\Spam")] {
        let mut folders = mailbox_memberships(folder, vec![label.into()], Some(&probe));
        retain_implicit_all_mail(&mut folders, &previous, &probe);
        assert!(!folders.contains(&"[Gmail]/All Mail".into()));
    }
    let mut fresh = vec!["INBOX".into()];
    retain_implicit_all_mail(&mut fresh, &[], &probe);
    assert_eq!(fresh, vec!["INBOX"]); // no invented, unobserved membership
}
#[test]
fn arrival_uses_provider_date_and_normalizes_timezone() {
    assert_eq!(
        arrival_time("28-Oct-2026 20:30:00 +0200").unwrap(),
        "2026-10-28T18:30:00.000Z"
    );
    assert_eq!(
        arrival_time(" 1-Jan-2000 00:00:00 -0500").unwrap(),
        "2000-01-01T05:00:00.000Z"
    );
    assert!(arrival_time("invalid").is_err());
}
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
struct FixtureProvider {
    account: String,
    fetches: AtomicUsize,
    probe_error: Option<p::ProviderError>,
    fetch_error: Option<p::ErrorKind>,
}
const MIME: &[u8] = b"From: Sender <sender@example.invalid>\r\nTo: Friend <friend@example.invalid>\r\nCc: Team <team@example.invalid>\r\nDate: Sat, 1 Jan 2000 00:00:00 +0000\r\nSubject: Synthetic fixture\r\nMessage-ID: <synthetic@example.invalid>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=fixture\r\n\r\n--fixture\r\nContent-Type: text/plain\r\n\r\nSynthetic only\r\n--fixture\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=example.txt\r\n\r\nattachment bytes\r\n--fixture--\r\n";
fn unsupported<T>() -> p::Result<T> {
    Err(p::error(p::ErrorKind::Unsupported, "fixture"))
}
#[async_trait]
impl p::MailProvider for FixtureProvider {
    fn account_id(&self) -> &str {
        &self.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        if let Some(error) = &self.probe_error {
            return Err(p::error(error.kind, &error.message));
        }
        Ok(p::Probe {
            capabilities: vec![],
            folders: vec![
                p::Folder {
                    name: "Sent Items".into(),
                    selectable: true,
                    attributes: vec!["Sent".into()],
                },
                p::Folder {
                    name: "INBOX".into(),
                    selectable: true,
                    attributes: vec![],
                },
            ],
            full_search_coverage: false,
        })
    }
    async fn scan(
        &self,
        folder: &str,
        _: &p::Search,
        _: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        let reference = p::MessageRef {
            graph_id: None,
            account_id: self.account.clone(),
            folder: folder.into(),
            uidvalidity: 1,
            uid: 1,
        };
        Ok(p::Page {
            rows: vec![p::Metadata {
                reference: reference.clone(),
                identity: p::Identity::Gmail {
                    account_id: self.account.clone(),
                    message_id: 1,
                },
                gmail_thread_id: Some(2),
                labels: vec![folder.into()],
                seen: false,
                starred: false,
                size: MIME.len() as u32,
                internal_date: "28-Oct-2026 20:30:00 +0200".into(),
                headers: MIME.to_vec(),
            }],
            next: None,
            uidvalidity: 1,
            full_coverage: false,
        })
    }
    async fn search(
        &self,
        folder: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        self.scan(folder, q, t, n).await
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        unsupported()
    }
    async fn fetch(&self, reference: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        assert_eq!(reference.account_id, self.account);
        self.fetches.fetch_add(1, Ordering::SeqCst);
        if reference.folder == "INBOX" {
            if let Some(kind) = self.fetch_error {
                return Err(p::error(kind, "fixture fetch failure"));
            }
        }
        Ok(MIME.to_vec())
    }
    async fn mutate(&self, _: &p::MessageRef, _: p::Mutation) -> p::Result<p::MutationResult> {
        unsupported()
    }
    async fn submit(
        &self,
        _: &p::mime::PreparedMessage,
        _: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        panic!("fixture must not send")
    }
    async fn reconcile_sent(&self, _: &str, _: &[u8]) -> p::Result<p::SentOutcome> {
        unsupported()
    }
}
#[tokio::test]
async fn sent_learning_arrival_and_forward_pin_use_authorized_provider() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [1; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"gmail","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let runtime = MailRuntime::default();
    let provider = Arc::new(FixtureProvider {
        account: a.id().into(),
        fetches: AtomicUsize::new(0),
        probe_error: None,
        fetch_error: None,
    });
    runtime
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    runtime.probe(&store, &a).await.unwrap();
    let page = runtime
        .page(&store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    let id = &page.rows[0].id;
    assert_eq!(page.rows[0].received_at, "2026-10-28T18:30:00.000Z");
    assert!(store
        .recipient_suggestions(&a, "")
        .await
        .unwrap()
        .is_empty());
    // A standard server may expose a folder without any special-use attribute.
    store
        .put_provider_document(
            &a,
            "probe",
            &serde_json::to_string(&p::Probe {
                capabilities: vec![],
                folders: vec![p::Folder {
                    name: "Sent Items".into(),
                    selectable: true,
                    attributes: vec![],
                }],
                full_search_coverage: false,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    store
        .put_provider_document(
            &a,
            "mailbox-settings",
            &serde_json::to_string(&MailboxSettings {
                sent_folder: Some("Sent Items".into()),
                ..Default::default()
            })
            .unwrap(),
        )
        .await
        .unwrap();
    runtime
        .page(&store, &a, "Sent Items", p::Search::default(), None)
        .await
        .unwrap();
    assert!(store
        .known_recipient(&a, "FRIEND@example.invalid")
        .await
        .unwrap());
    assert!(store
        .known_recipient(&a, "team@example.invalid")
        .await
        .unwrap());
    assert!(!store
        .known_recipient(&a, "sender@example.invalid")
        .await
        .unwrap());
    let content = runtime.content(&store, &a, id).await.unwrap();
    // Hydration records unsupported lists accurately and the content cache does
    // not trigger another provider read or change mail flags.
    assert_eq!(
        store
            .provider_document(&a, &format!("unsubscribe-v1:{id}"))
            .await
            .unwrap()
            .as_deref(),
        Some("null")
    );
    let fetches = provider.fetches.load(Ordering::SeqCst);
    runtime.content(&store, &a, id).await.unwrap();
    assert_eq!(provider.fetches.load(Ordering::SeqCst), fetches);
    let part = &content.attachments[0];
    assert!(store.attachment(&a, &part.id).await.is_err());
    let pinned = runtime
        .pin_received_attachment(&store, &a, &part.id)
        .await
        .unwrap();
    assert_eq!(&pinned, part);
    assert_eq!(
        store.attachment(&a, &part.id).await.unwrap().1,
        b"attachment bytes"
    );
    let fetched = provider.fetches.load(Ordering::SeqCst);
    runtime
        .pin_received_attachment(&store, &a, &part.id)
        .await
        .unwrap();
    assert_eq!(provider.fetches.load(Ordering::SeqCst), fetched);
    let other_user = store.user("fixture", "other").await.unwrap();
    let other = store.connect(&other_user, &input).await.unwrap();
    let other = store.authorize(&other_user, &other.id).await.unwrap();
    assert!(runtime
        .pin_received_attachment(&store, &other, &part.id)
        .await
        .is_err());
    assert!(!store
        .known_recipient(&other, "friend@example.invalid")
        .await
        .unwrap());
    let mut settings = MailboxSettings {
        sent_folder: Some("not discovered".into()),
        ..Default::default()
    };
    assert!(runtime
        .configure(&store, &a, settings.clone())
        .await
        .is_err());
    settings.sent_folder = Some("Sent Items".into());
    settings.full_imap_exposure_confirmed = true;
    settings.received_cache_budget = 512 * 1024 * 1024;
    assert_eq!(
        runtime
            .configure(&store, &a, settings.clone())
            .await
            .unwrap(),
        settings
    );
    assert_eq!(runtime.settings(&store, &a).await.unwrap(), settings);
    assert_eq!(
        runtime.settings(&store, &other).await.unwrap(),
        MailboxSettings::default()
    );
    assert!(!runtime.providers.lock().await.contains_key(a.id()));
}

#[tokio::test]
async fn gmail_setup_http_probe_failure_reconnect_and_recovery() {
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [3; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(URL_SAFE_NO_PAD.encode(Sha256::digest(b"setup-fixture-token")))
        .bind(&user)
        .bind("setup-csrf")
        .bind(4102444800_i64)
        .execute(store.control())
        .await
        .unwrap();
    let app = crate::App {
        store,
        config: Arc::new(crate::Config {
            origin: "https://dakia.example.invalid".into(),
            issuer: "https://id.example.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture-only".into(),
            web_root: dir.path().to_string_lossy().into(),
        }),
        mail: MailRuntime::default(),
    };
    let request = |method: &str, path: &str, body: String| {
        Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", "__Host-dakia=setup-fixture-token")
            .header("origin", "https://dakia.example.invalid")
            .header("x-csrf-token", "setup-csrf")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    let mut input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"gmail","imap_host":"imap.gmail.com","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.gmail.com","smtp_port":465,"smtp_tls":"implicit","username":"owner@example.invalid","password":"fixture-only"})).unwrap();
    let response = crate::router(app.clone())
        .oneshot(request(
            "POST",
            "/api/accounts",
            serde_json::to_string(&input).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let account: Account =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    let scope = app.store.authorize(&user, &account.id).await.unwrap();
    let path = format!("/api/accounts/{}", account.id);
    let probe_path = format!("{path}/probe");

    for (kind, message, code, state) in [
        (
            p::ErrorKind::Authentication,
            "IMAP probe: private provider reply",
            "imap_authentication_rejected",
            AccountState::Reconnect,
        ),
        (
            p::ErrorKind::Authentication,
            "SMTP probe: private provider reply",
            "smtp_authentication_rejected",
            AccountState::Reconnect,
        ),
        (
            p::ErrorKind::Retryable,
            "IMAP probe: private network detail",
            "provider_unavailable",
            AccountState::Error,
        ),
    ] {
        app.mail.providers.lock().await.insert(
            account.id.clone(),
            Arc::new(FixtureProvider {
                account: account.id.clone(),
                fetches: AtomicUsize::new(0),
                probe_error: Some(p::error(kind, message)),
                fetch_error: None,
            }),
        );
        let response = crate::router(app.clone())
            .oneshot(request("POST", &probe_path, String::new()))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if kind == p::ErrorKind::Authentication {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_GATEWAY
            }
        );
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let error: ApiError = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error.code, code);
        assert!(!String::from_utf8_lossy(&bytes).contains("private"));
        assert_eq!(app.store.accounts(&user).await.unwrap()[0].state, state);

        input.password = "replacement-fixture-only".into();
        let response = crate::router(app.clone())
            .oneshot(request(
                "PUT",
                &path,
                serde_json::to_string(&input).unwrap(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!app.mail.providers.lock().await.contains_key(&account.id));
        assert_eq!(app.store.accounts(&user).await.unwrap().len(), 1);
        assert_eq!(
            app.store.credential(&scope).await.unwrap().password,
            input.password
        );
    }
    app.mail.providers.lock().await.insert(
        account.id.clone(),
        Arc::new(FixtureProvider {
            account: account.id.clone(),
            fetches: AtomicUsize::new(0),
            probe_error: None,
            fetch_error: None,
        }),
    );
    let response = crate::router(app.clone())
        .oneshot(request("POST", &probe_path, String::new()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ready: Account =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(ready.id, account.id);
    assert_eq!(ready.state, AccountState::Ready);
    assert_eq!(app.store.accounts(&user).await.unwrap().len(), 1);
}

#[tokio::test]
async fn gmail_stale_membership_fetch_falls_back_but_authentication_does_not() {
    for kind in [p::ErrorKind::StaleReference, p::ErrorKind::Authentication] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), [4; 32]).await.unwrap();
        let user = store.user("fixture", "owner").await.unwrap();
        let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"gmail","imap_host":"imap.gmail.com","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.gmail.com","smtp_port":465,"smtp_tls":"implicit","username":"owner@example.invalid","password":"fixture-only"})).unwrap();
        let account = store.connect(&user, &input).await.unwrap();
        let a = store.authorize(&user, &account.id).await.unwrap();
        let runtime = MailRuntime::default();
        let provider = Arc::new(FixtureProvider {
            account: account.id.clone(),
            fetches: AtomicUsize::new(0),
            probe_error: None,
            fetch_error: Some(kind),
        });
        runtime
            .providers
            .lock()
            .await
            .insert(account.id.clone(), provider.clone());
        let archive = runtime
            .page(&store, &a, "All Mail", p::Search::default(), None)
            .await
            .unwrap();
        let inbox = runtime
            .page(&store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap();
        let id = &inbox.rows[0].id;
        assert_eq!(*id, archive.rows[0].id);
        // A corrupt foreign-account candidate must never reach the provider.
        let foreign = p::MessageRef {
            graph_id: None,
            account_id: "foreign-account".into(),
            folder: "Other".into(),
            uidvalidity: 1,
            uid: 1,
        };
        store
            .put_provider_document(
                &a,
                &format!("member-reference:{id}:!foreign"),
                &serde_json::to_string(&foreign).unwrap(),
            )
            .await
            .unwrap();
        let raw = runtime.raw(&store, &a, id).await;
        if kind == p::ErrorKind::StaleReference {
            assert_eq!(raw.unwrap(), MIME);
            assert_eq!(provider.fetches.load(Ordering::SeqCst), 2);
            let reference: p::MessageRef = serde_json::from_str(
                &store
                    .provider_document(&a, &format!("reference:{id}"))
                    .await
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(reference.folder, "All Mail");
            assert_eq!(runtime.raw(&store, &a, id).await.unwrap(), MIME);
            assert_eq!(provider.fetches.load(Ordering::SeqCst), 3);
        } else {
            assert_eq!(raw.unwrap_err().0, StatusCode::UNAUTHORIZED);
            assert_eq!(provider.fetches.load(Ordering::SeqCst), 1);
        }
        let other_user = store.user("fixture", "other").await.unwrap();
        let other_account = store.connect(&other_user, &input).await.unwrap();
        let other = store
            .authorize(&other_user, &other_account.id)
            .await
            .unwrap();
        assert!(store
            .message_reference_documents(&other, id)
            .await
            .unwrap()
            .is_empty());
        assert!(runtime.raw(&store, &other, id).await.is_err());
    }
}

#[tokio::test]
async fn oversized_content_is_an_actionable_permanent_error_not_a_provider_outage() {
    use axum::response::IntoResponse;
    let response = provider_error(p::error(
        p::ErrorKind::InvalidInput,
        "message exceeds content limit",
    ))
    .into_response();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "message_too_large");
    assert_eq!(
        error["message"],
        "This message is too large to open here. Open it in your mail provider."
    );
}
