//! Synthetic provider exercising the real list/store/worker path, never owner mail.
use super::*;
use async_trait::async_trait;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;

struct ArchiveProvider {
    account: String,
    gmail: bool,
    archived: Mutex<BTreeSet<u32>>,
    fail: AtomicBool,
    pause: AtomicBool,
    entered: Notify,
    release: Notify,
    mutations: AtomicUsize,
    uncertain: AtomicBool,
    verified: AtomicBool,
    readbacks: AtomicUsize,
}
#[async_trait]
impl p::MailProvider for ArchiveProvider {
    fn account_id(&self) -> &str {
        &self.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        Ok(p::Probe {
            capabilities: vec![],
            folders: self.folders().await?,
            full_search_coverage: true,
        })
    }
    async fn folders(&self) -> p::Result<Vec<p::Folder>> {
        Ok([
            ("INBOX", "Inbox"),
            ("Archive", "All"),
            ("Junk", "Junk"),
            ("Trash", "Trash"),
        ]
        .into_iter()
        .map(|(name, role)| p::Folder {
            name: name.into(),
            selectable: true,
            attributes: vec![role.into()],
        })
        .collect())
    }
    async fn scan(
        &self,
        folder: &str,
        _: &p::Search,
        token: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        let archived = self.archived.lock().await.clone();
        let second = token.is_some();
        let rows = (if second { 3..=4 } else { 1..=2 }).filter(|uid| if folder == "INBOX" { !archived.contains(uid) } else { self.gmail || archived.contains(uid) }).map(|uid| {
            let reference = p::MessageRef { account_id: self.account.clone(), folder: folder.into(), uidvalidity: 1, uid };
            p::Metadata { identity: if self.gmail { p::Identity::Gmail { account_id: self.account.clone(), message_id: uid.into() } } else { p::Identity::Imap(reference.clone()) }, reference, gmail_thread_id: None,
                labels: if self.gmail && !archived.contains(&uid) { vec!["\\Inbox".into(), "Important".into()] } else { vec![] },
                seen: false, starred: false, size: 100, internal_date: "28-Oct-2026 20:30:00 +0200".into(),
                headers: format!("From: fixture@example.invalid\r\nSubject: Archive {uid}\r\nMessage-ID: <archive-{uid}@example.invalid>\r\n\r\n").into_bytes() }
        }).collect();
        if self.pause.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(p::Page {
            rows,
            next: (!second).then(|| p::PageToken {
                account_id: self.account.clone(),
                folder: folder.into(),
                uidvalidity: 1,
                upper_uid: 4,
                before_uid: 3,
                query: p::Search::default(),
                modseq: None,
            }),
            uidvalidity: 1,
            full_coverage: true,
        })
    }
    async fn search(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        self.scan(f, q, t, n).await
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        Ok(false)
    }
    async fn fetch(&self, _: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        panic!("no content needed")
    }
    async fn verify_mutation(
        &self,
        r: &p::MessageRef,
        identity: &p::Identity,
        _: &p::Mutation,
    ) -> p::Result<Option<p::MutationResult>> {
        self.readbacks.fetch_add(1, Ordering::SeqCst);
        assert!(
            matches!(identity, p::Identity::Gmail { account_id, message_id } if account_id == &self.account && *message_id == u64::from(r.uid))
        );
        Ok(self
            .verified
            .load(Ordering::SeqCst)
            .then(|| p::MutationResult {
                destination: Some("Archive".into()),
                destination_reference: Some(p::MessageRef {
                    account_id: r.account_id.clone(),
                    folder: "Archive".into(),
                    uidvalidity: 1,
                    uid: r.uid,
                }),
                requires_rescan: true,
            }))
    }
    async fn mutate(
        &self,
        r: &p::MessageRef,
        mutation: p::Mutation,
    ) -> p::Result<p::MutationResult> {
        assert_eq!(r.account_id, self.account);
        let spam = matches!(mutation, p::Mutation::Spam);
        let trash = matches!(mutation, p::Mutation::Trash);
        if !spam && !trash {
            assert_eq!(r.folder, "INBOX");
        }
        assert!(matches!(
            mutation,
            p::Mutation::Archive | p::Mutation::Spam | p::Mutation::Trash
        ));
        self.mutations.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(p::error(
                p::ErrorKind::Unsupported,
                "synthetic rejection private@example.invalid https://private.example.invalid/token",
            ));
        }
        if self.uncertain.load(Ordering::SeqCst) {
            return Err(p::error(
                p::ErrorKind::Uncertain,
                "IMAP connection closed before tagged completion",
            ));
        }
        self.archived.lock().await.insert(r.uid);
        let destination = if spam {
            "Junk"
        } else if trash {
            "Trash"
        } else {
            "Archive"
        };
        Ok(p::MutationResult {
            requires_rescan: true,
            destination: Some(destination.into()),
            destination_reference: Some(p::MessageRef {
                folder: destination.into(),
                ..r.clone()
            }),
        })
    }
    async fn submit(
        &self,
        _: &p::mime::PreparedMessage,
        _: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        panic!("must not send")
    }
    async fn reconcile_sent(&self, _: &str, _: &[u8]) -> p::Result<p::SentOutcome> {
        panic!("must not file")
    }
}
async fn fixture(
    gmail: bool,
) -> (
    tempfile::TempDir,
    crate::App,
    AuthorizedAccount,
    Arc<ArchiveProvider>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [8; 32]).await.unwrap();
    let user = store.user("fixture", "archive-owner").await.unwrap();
    let input = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":if gmail {"gmail"}else{"imap"},"imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"synthetic-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let mail = MailRuntime::default();
    let provider = Arc::new(ArchiveProvider {
        account: a.id().into(),
        gmail,
        archived: Mutex::default(),
        fail: AtomicBool::new(false),
        pause: AtomicBool::new(false),
        entered: Notify::new(),
        release: Notify::new(),
        mutations: AtomicUsize::new(0),
        uncertain: AtomicBool::new(false),
        verified: AtomicBool::new(false),
        readbacks: AtomicUsize::new(0),
    });
    mail.providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    mail.probe(&store, &a).await.unwrap();
    let app = crate::App {
        store,
        mail,
        config: Arc::new(crate::Config {
            origin: "https://dakia.example.invalid".into(),
            issuer: "https://id.example.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "synthetic".into(),
            web_root: dir.path().to_string_lossy().into(),
        }),
    };
    (dir, app, a, provider)
}
async fn queue(app: &crate::App, a: &AuthorizedAccount, id: &str, name: &str) -> Operation {
    let m = app.store.message(a, id).await.unwrap();
    app.store
        .operation(
            a,
            Operation {
                id: name.into(),
                account_id: a.id().into(),
                target_id: id.into(),
                expected_revision: m.revision,
                change: Change::Archive,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn queued_restore_after_a_failed_archive_does_not_move_inbox_onto_itself() {
    for gmail in [false, true] {
        let (_dir, app, a, provider) = fixture(gmail).await;
        let first = app
            .mail
            .page(&app.store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap();
        let id = &first.rows[0].id;
        let archive = queue(&app, &a, id, "earlier-archive").await;
        let restore = app
            .store
            .operation(
                &a,
                Operation {
                    id: "later-restore".into(),
                    account_id: a.id().into(),
                    target_id: id.clone(),
                    expected_revision: archive.revision,
                    change: Change::Move {
                        folder_id: "INBOX".into(),
                    },
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(restore.status, OperationStatus::Pending);
        provider.fail.store(true, Ordering::SeqCst);
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(
            app.store
                .operation_status(&a, &archive.id)
                .await
                .unwrap()
                .status,
            OperationStatus::Failed
        );
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(
            app.store
                .operation_status(&a, &restore.id)
                .await
                .unwrap()
                .status,
            OperationStatus::Applied
        );
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
        assert!(app
            .store
            .message(&a, id)
            .await
            .unwrap()
            .folder_ids
            .iter()
            .any(|folder| folder == "INBOX"));
        assert!(app.store.claim_mutation(&a).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn sequential_and_concurrent_archives_refetch_pagination_and_failure() {
    for gmail in [false, true] {
        let (_dir, app, a, provider) = fixture(gmail).await;
        let first = app
            .mail
            .page(&app.store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap();
        let ids: Vec<_> = first.rows.iter().map(|m| m.id.clone()).collect();
        let second = app
            .mail
            .page(
                &app.store,
                &a,
                "INBOX",
                p::Search::default(),
                first.continuation.as_deref(),
            )
            .await
            .unwrap();
        let third = second.rows[0].id.clone();
        // A later Gmail label scan must not select an Archive UID as the source.
        if gmail {
            app.mail
                .page(&app.store, &a, "Archive", p::Search::default(), None)
                .await
                .unwrap();
        }
        let op = queue(&app, &a, &ids[0], "first").await;
        assert_eq!(op.status, OperationStatus::Pending);
        let pending = app
            .mail
            .page(&app.store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap();
        assert!(!pending.rows.iter().any(|m| m.id == ids[0]));
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(
            app.store.operation_status(&a, &op.id).await.unwrap().status,
            OperationStatus::Applied
        );
        queue(&app, &a, &ids[1], "second").await;
        queue(&app, &a, &third, "third").await;
        let (x, y) = tokio::join!(
            crate::mutations::tick(&app, &a),
            crate::mutations::tick(&app, &a)
        );
        x.unwrap();
        y.unwrap();
        let fresh = app
            .mail
            .page(&app.store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap();
        assert!(fresh.rows.is_empty());
        let fresh = app
            .mail
            .page(
                &app.store,
                &a,
                "INBOX",
                p::Search::default(),
                fresh.continuation.as_deref(),
            )
            .await
            .unwrap();
        assert_eq!(fresh.rows.len(), 1);
        let fourth = fresh.rows[0].id.clone();
        provider.fail.store(true, Ordering::SeqCst);
        let failed = queue(&app, &a, &fourth, "failed").await;
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(
            app.store
                .operation_status(&a, &failed.id)
                .await
                .unwrap()
                .status,
            OperationStatus::Failed
        );
        let recovered = app
            .mail
            .page(
                &app.store,
                &a,
                "INBOX",
                p::Search::default(),
                first.continuation.as_deref(),
            )
            .await
            .unwrap();
        assert_eq!(recovered.rows[0].id, fourth);
        let archived = app
            .mail
            .page(&app.store, &a, "Archive", p::Search::default(), None)
            .await
            .unwrap();
        assert!(archived.rows.iter().any(|m| m.id == ids[0]));
        assert!(!app
            .store
            .message(&a, &ids[0])
            .await
            .unwrap()
            .folder_ids
            .contains(&"INBOX".into()));
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 4);
    }
}
#[tokio::test]
async fn old_scan_cannot_ingest_after_archive_completion_and_lanes_are_account_scoped() {
    let (_dir, app, a, provider) = fixture(true).await;
    let page = app
        .mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    queue(&app, &a, &page.rows[0].id, "archive").await;
    provider.pause.store(true, Ordering::SeqCst);
    let scan = {
        let app = app.clone();
        let a = a.clone();
        tokio::spawn(async move {
            app.mail
                .page(&app.store, &a, "INBOX", p::Search::default(), None)
                .await
        })
    };
    provider.entered.notified().await;
    let worker = {
        let app = app.clone();
        let a = a.clone();
        tokio::spawn(async move { crate::mutations::tick(&app, &a).await })
    };
    tokio::task::yield_now().await;
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
    provider.release.notify_one();
    scan.await.unwrap().unwrap();
    worker.await.unwrap().unwrap();
    assert!(!app
        .store
        .message(&a, &page.rows[0].id)
        .await
        .unwrap()
        .folder_ids
        .contains(&"INBOX".into()));
    let input = app.store.credential(&a).await.unwrap();
    let other_user = app.store.user("fixture", "other").await.unwrap();
    let other = app.store.connect(&other_user, &input).await.unwrap();
    let other = app.store.authorize(&other_user, &other.id).await.unwrap();
    assert!(!Arc::ptr_eq(
        &app.mail.reconciliation_lane(&a).await,
        &app.mail.reconciliation_lane(&other).await
    ));
    assert!(app.store.operation_status(&other, "archive").await.is_err());
}

#[tokio::test]
async fn spam_queue_pending_success_failure_identity_and_account_scope() {
    for gmail in [false, true] {
        let (_dir, app, a, provider) = fixture(gmail).await;
        let page = app
            .mail
            .page(&app.store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap();
        let id = &page.rows[0].id;
        if gmail {
            // A later All Mail scan can own the canonical UID. Spam must still
            // clear local Inbox/All Mail, without removing unrelated labels.
            app.mail
                .page(&app.store, &a, "Archive", p::Search::default(), None)
                .await
                .unwrap();
        }
        let message = app.store.message(&a, id).await.unwrap();
        let request = Operation {
            id: "spam".into(),
            account_id: a.id().into(),
            target_id: id.clone(),
            expected_revision: message.revision,
            change: Change::Spam,
            status: OperationStatus::Pending,
            revision: 0,
        };
        let op = app.store.operation(&a, request.clone()).await.unwrap();
        assert_eq!(op.status, OperationStatus::Pending);
        assert_eq!(app.store.operation(&a, request.clone()).await.unwrap(), op);
        assert!(!app
            .store
            .inbox_rows(&a, None, None)
            .await
            .unwrap()
            .iter()
            .any(|m| &m.id == id));
        assert!(!app
            .store
            .visible_message(&a, message)
            .await
            .unwrap()
            .folder_ids
            .contains(&"INBOX".into()));
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(
            app.store.operation_status(&a, "spam").await.unwrap().status,
            OperationStatus::Applied
        );
        let moved = app.store.message(&a, id).await.unwrap();
        assert!(moved.folder_ids.contains(&"Junk".into()));
        assert!(!moved.folder_ids.contains(&"INBOX".into()));
        if gmail {
            assert!(!moved.folder_ids.contains(&"Archive".into()));
            assert!(moved.folder_ids.contains(&"Important".into()));
        }
        let reference: p::MessageRef = serde_json::from_str(
            &app.store
                .provider_document(&a, &format!("reference:{id}"))
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(reference.folder, "Junk");
        assert_eq!(reference.account_id, a.id());
        let junk = app
            .mail
            .page(&app.store, &a, "Junk", p::Search::default(), None)
            .await
            .unwrap();
        assert!(junk.rows.iter().any(|m| &m.id == id));
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);

        let second = &page.rows[1].id;
        let message = app.store.message(&a, second).await.unwrap();
        app.store
            .operation(
                &a,
                Operation {
                    id: "failed-spam".into(),
                    target_id: second.clone(),
                    expected_revision: message.revision,
                    ..request
                },
            )
            .await
            .unwrap();
        provider.fail.store(true, Ordering::SeqCst);
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(
            app.store
                .operation_status(&a, "failed-spam")
                .await
                .unwrap()
                .status,
            OperationStatus::Failed
        );
        assert!(app
            .store
            .inbox_rows(&a, None, None)
            .await
            .unwrap()
            .iter()
            .any(|m| &m.id == second));
        let input = app.store.credential(&a).await.unwrap();
        let other_user = app.store.user("fixture", "spam-other").await.unwrap();
        let other = app.store.connect(&other_user, &input).await.unwrap();
        let other = app.store.authorize(&other_user, &other.id).await.unwrap();
        assert!(app.store.operation_status(&other, "spam").await.is_err());
        assert!(app.store.message(&other, id).await.is_err());
    }
}

#[tokio::test]
async fn trash_from_canonical_all_mail_removes_inbox_and_preserves_unselected_sibling() {
    for gmail in [false, true] {
        let (_dir, app, a, provider) = fixture(gmail).await;
        let page = app
            .mail
            .page(&app.store, &a, "INBOX", p::Search::default(), None)
            .await
            .unwrap();
        if gmail {
            app.mail
                .page(&app.store, &a, "Archive", p::Search::default(), None)
                .await
                .unwrap();
        }
        let m = app.store.message(&a, &page.rows[0].id).await.unwrap();
        let sibling = app.store.message(&a, &page.rows[1].id).await.unwrap();
        let op = Operation {
            id: "trash-explicit-target".into(),
            account_id: a.id().into(),
            target_id: m.id.clone(),
            expected_revision: m.revision,
            change: Change::Trash,
            status: OperationStatus::Pending,
            revision: 0,
        };
        app.store.operation(&a, op.clone()).await.unwrap();
        app.store.operation(&a, op).await.unwrap();
        crate::mutations::tick(&app, &a).await.unwrap();
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
        assert_eq!(provider.archived.lock().await.len(), 1);
        let moved = app.store.message(&a, &m.id).await.unwrap();
        assert!(moved.folder_ids.contains(&"Trash".into()));
        assert!(!moved
            .folder_ids
            .iter()
            .any(|folder| folder.eq_ignore_ascii_case("INBOX")
                || folder == "\\Inbox"
                || folder == "Archive"));
        if gmail {
            assert!(moved.folder_ids.contains(&"Important".into()));
        }
        assert_eq!(app.store.message(&a, &sibling.id).await.unwrap(), sibling);
        assert_eq!(
            app.store
                .operation_status(&a, "trash-explicit-target")
                .await
                .unwrap()
                .status,
            OperationStatus::Applied
        );
    }
}

#[tokio::test]
async fn mutation_failure_is_correlated_without_private_response_or_replay() {
    // Tracing callsite interest is process-global. Isolate capture from tests
    // concurrently invoking the same provider mapper without a subscriber.
    const CHILD: &str = "DAKIA_MUTATION_LOG_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "mail::archive_tests::mutation_failure_is_correlated_without_private_response_or_replay", "--nocapture"])
            .env(CHILD, "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use std::io::Write;
    use tracing::instrument::WithSubscriber;
    #[derive(Clone, Default)]
    struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let (_dir, app, a, provider) = fixture(true).await;
    let page = app
        .mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    let op = queue(
        &app,
        &a,
        &page.rows[0].id,
        "private-client-id@example.invalid",
    )
    .await;
    provider.fail.store(true, Ordering::SeqCst);
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    crate::mutations::tick(&app, &a)
        .with_subscriber(subscriber)
        .await
        .unwrap();
    crate::mutations::tick(&app, &a).await.unwrap();
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
    assert_eq!(
        app.store.operation_status(&a, &op.id).await.unwrap().status,
        OperationStatus::Failed
    );
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for expected in [
        "account_key=",
        "operation_key=",
        "action=\"archive\"",
        "mutate_or_verify",
        "kind=Unsupported",
        "reason=\"other\"",
        "outcome persisted",
        "status=Failed",
        "elapsed_ms=",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    for private in [
        "private-client-id",
        "private@example.invalid",
        "https://",
        "synthetic rejection",
        a.id(),
        &page.rows[0].subject,
    ] {
        assert!(!text.contains(private), "private value leaked");
    }
}

#[tokio::test]
async fn uncertain_archive_readback_updates_local_state_only_when_proven_and_never_replays() {
    for verified in [false, true] {
        let (_dir, app, a, p) = fixture(true).await;
        let page = app
            .mail
            .snapshot_page(&app.store, &a, "INBOX", None)
            .await
            .unwrap();
        let message = &page.rows[0];
        p.uncertain.store(true, Ordering::SeqCst);
        p.verified.store(verified, Ordering::SeqCst);
        let op = queue(&app, &a, &message.id, "lost-ack").await;
        crate::mutations::tick(&app, &a).await.unwrap();
        crate::mutations::tick(&app, &a).await.unwrap();
        assert_eq!(p.mutations.load(Ordering::SeqCst), 1);
        assert_eq!(p.readbacks.load(Ordering::SeqCst), 1);
        let result = app.store.operation_status(&a, &op.id).await.unwrap();
        assert_eq!(
            result.status,
            if verified {
                OperationStatus::Applied
            } else {
                OperationStatus::Failed
            }
        );
        let message = app.store.message(&a, &message.id).await.unwrap();
        assert_eq!(message.folder_ids.iter().any(|f| f == "INBOX"), !verified);
    }
}
