use dakia_domain::*;
use dakia_storage::{Error, Store};

fn account() -> ConnectAccount {
    ConnectAccount {
        email: "test@example.invalid".into(),
        display_name: "Test".into(),
        provider: ProviderKind::Imap,
        imap_host: "imap.example.invalid".into(),
        imap_port: 993,
        imap_tls: TlsMode::Implicit,
        smtp_host: "smtp.example.invalid".into(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        username: "test".into(),
        password: "test-only-not-a-real-secret".into(),
        gmail_oauth: None,
    }
}
#[tokio::test]
async fn coordinated_backup_restores_uncertain_without_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path().join("live"), [42; 32])
        .await
        .unwrap();
    let user = s.user("issuer", "subject").await.unwrap();
    let a = s.connect(&user, &account()).await.unwrap();
    let a = s.authorize(&user, &a.id).await.unwrap();
    let attachment = s
        .put_attachment(
            &a,
            "../../not-a-path.txt".into(),
            "text/plain".into(),
            b"synthetic attachment",
        )
        .await
        .unwrap();
    let mut input = draft(a.id());
    input.attachments.push(attachment.clone());
    let d = s.save_draft(&a, input).await.unwrap();
    let r = SubmitRequest {
        operation_id: "backup-op".into(),
        draft_id: d.id.clone(),
        expected_revision: d.revision,
        due_at: None,
        timezone: "UTC".into(),
    };
    let job = s.submit(&a, &r).await.unwrap();
    s.transition(
        &a,
        &job.id,
        SubmissionState::Scheduled,
        SubmissionState::Claimed,
    )
    .await
    .unwrap();
    s.transition(
        &a,
        &job.id,
        SubmissionState::Claimed,
        SubmissionState::Submitting,
    )
    .await
    .unwrap();
    s.backup(dir.path().join("backup")).await.unwrap();
    let restored = Store::restore(
        dir.path().join("backup"),
        dir.path().join("restored"),
        [42; 32],
    )
    .await
    .unwrap();
    assert!(!restored.jobs_enabled().await.unwrap());
    let a2 = restored.authorize(&user, a.id()).await.unwrap();
    assert_eq!(restored.draft(&a2, &d.id).await.unwrap(), d);
    assert_eq!(
        restored.attachment(&a2, &attachment.id).await.unwrap().1,
        b"synthetic attachment"
    );
    assert_eq!(
        restored.submission(&a2, &job.id).await.unwrap().state,
        SubmissionState::DeliveryUnknown
    );
    assert_eq!(
        s.submission(&a, &job.id).await.unwrap().state,
        SubmissionState::Submitting
    );
    assert!(Store::restore(
        dir.path().join("backup"),
        dir.path().join("restored"),
        [42; 32]
    )
    .await
    .is_err());
}

#[tokio::test]
async fn operation_dedup_reordering_and_manual_category_survive_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path(), [42; 32]).await.unwrap();
    let u = s.user("issuer", "subject").await.unwrap();
    let a = s.connect(&u, &account()).await.unwrap();
    let a = s.authorize(&u, &a.id).await.unwrap();
    let m = Message {
        id: "message".into(),
        account_id: a.id().into(),
        thread_id: "thread".into(),
        provider_id: "provider-id".into(),
        folder_ids: vec!["INBOX".into()],
        sender: Address {
            email: "test@example.invalid".into(),
            name: None,
        },
        to: vec![],
        subject: "test".into(),
        preview: "test".into(),
        received_at: "2026-10-28T00:00:00Z".into(),
        unread: true,
        starred: false,
        category: None,
        category_status: CategoryStatus::Pending,
        revision: 0,
    };
    let m = s.upsert_message(&a, m).await.unwrap();
    let op = Operation {
        id: "op".into(),
        account_id: a.id().into(),
        target_id: m.id.clone(),
        expected_revision: m.revision,
        change: Change::Star { value: true },
        status: OperationStatus::Pending,
        revision: 0,
    };
    let applied = s.operation(&a, op.clone()).await.unwrap();
    assert_eq!(s.operation(&a, op.clone()).await.unwrap(), applied);
    let mut old = op.clone();
    old.id = "old".into();
    assert!(matches!(s.operation(&a, old).await, Err(Error::Conflict)));
    let mut correction = op;
    correction.id = "category".into();
    correction.expected_revision = applied.revision;
    correction.change = Change::Category {
        value: "people".into(),
    };
    s.operation(&a, correction).await.unwrap();
    let refreshed = s.upsert_message(&a, m).await.unwrap();
    assert!(
        refreshed.starred,
        "stale provider metadata cannot undo a pending newer click"
    );
    assert_eq!(refreshed.category.as_deref(), Some("people"));
    assert_eq!(refreshed.category_status, CategoryStatus::Ready);
    let stable = s.message_id(&a, "gmail:12345").await.unwrap();
    assert_eq!(stable, s.message_id(&a, "gmail:12345").await.unwrap());
    let content = MessageContent {
        reply_to: vec![],
        detected_language: None,
        message_id: None,
        references: vec![],
        cc: vec![],
        message: refreshed.clone(),
        text: "synthetic body".into(),
        html: "<p>synthetic body</p>".into(),
        attachments: vec![],
    };
    let mut legacy = serde_json::to_value(&content).unwrap();
    legacy.as_object_mut().unwrap().remove("detected_language");
    let restored: MessageContent = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.detected_language, None);
    s.cache_content(&a, &content, 1024 * 1024).await.unwrap();
    assert_eq!(
        s.cached_content(&a, &refreshed.id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "synthetic body"
    );
    // Upgrade a legacy on-disk body cache only when the owner opens it. A
    // missing Reply-To field is not evidence that the message has no Reply-To.
    let cache_directory = dir.path().join("cache").join(a.id());
    let blob = std::fs::read_dir(&cache_directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut legacy_cache = serde_json::to_value(&content).unwrap();
    legacy_cache.as_object_mut().unwrap().remove("reply_to");
    std::fs::write(blob, serde_json::to_vec(&legacy_cache).unwrap()).unwrap();
    assert!(s.cached_content(&a, &refreshed.id).await.unwrap().is_none());
    s.cache_content(&a, &content, 1024 * 1024).await.unwrap();
    assert!(s.cached_content(&a, &refreshed.id).await.unwrap().is_some());
    s.save_draft(&a, draft(a.id())).await.unwrap();
    s.evict_received(&a).await.unwrap();
    assert!(s.cached_content(&a, &refreshed.id).await.unwrap().is_none());
    assert_eq!(
        s.message(&a, &refreshed.id)
            .await
            .unwrap()
            .category
            .as_deref(),
        Some("people")
    );
    assert!(s.draft(&a, "draft-1").await.is_ok());
}

#[tokio::test]
async fn due_claim_immutable_mime_and_filing_are_separate() {
    use dakia_storage::PersistedMime;
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path(), [42; 32]).await.unwrap();
    let user = s.user("issuer", "subject").await.unwrap();
    let a = s.connect(&user, &account()).await.unwrap();
    let a = s.authorize(&user, &a.id).await.unwrap();
    let d = s.save_draft(&a, draft(a.id())).await.unwrap();
    let job = s
        .submit(
            &a,
            &SubmitRequest {
                operation_id: "due-op".into(),
                draft_id: d.id.clone(),
                expected_revision: d.revision,
                due_at: Some("2090-01-01T00:00:00Z".into()),
                timezone: "UTC".into(),
            },
        )
        .await
        .unwrap();
    assert!(s
        .due_submission(&a, "2089-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        s.due_submission(&a, "2091-01-01T00:00:00.000Z")
            .await
            .unwrap()
            .unwrap()
            .state,
        SubmissionState::Claimed
    );
    assert!(s
        .due_submission(&a, "2091-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .is_none());
    assert_eq!(s.submission_snapshot(&a, &job.id).await.unwrap(), d);
    let mime = PersistedMime {
        raw: b"synthetic MIME fixture".to_vec(),
        recipients: vec!["test@example.invalid".into()],
        message_id: "<test@example.invalid>".into(),
    };
    s.persist_mime(&a, &job.id, mime).await.unwrap();
    let unchanged = s
        .persist_mime(
            &a,
            &job.id,
            PersistedMime {
                raw: b"must not replace".to_vec(),
                recipients: vec![],
                message_id: "other".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(unchanged.raw, b"synthetic MIME fixture");
    s.transition(
        &a,
        &job.id,
        SubmissionState::Claimed,
        SubmissionState::Submitting,
    )
    .await
    .unwrap();
    s.transition(
        &a,
        &job.id,
        SubmissionState::Submitting,
        SubmissionState::Accepted,
    )
    .await
    .unwrap();
    assert_eq!(s.needs_filing(&a).await.unwrap().len(), 1);
    let job = s
        .set_filing(&a, &job.id, FilingStatus::Failed)
        .await
        .unwrap();
    assert_eq!(job.state, SubmissionState::Accepted);
    assert!(s
        .due_submission(&a, "2091-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .is_none());
    s.set_filing(&a, &job.id, FilingStatus::Filed)
        .await
        .unwrap();
    assert!(s.needs_filing(&a).await.unwrap().is_empty());
}

fn draft(account: &str) -> Draft {
    Draft {
        id: "draft-1".into(),
        account_id: account.into(),
        revision: 0,
        to: vec![Address {
            email: "recipient@example.invalid".into(),
            name: None,
        }],
        cc: vec![],
        bcc: vec![],
        subject: "Synthetic".into(),
        text: "Test".into(),
        html: "<p>Test</p>".into(),
        in_reply_to: None,
        references: vec![],
        attachments: vec![],
        saved_at: String::new(),
    }
}
#[tokio::test]
async fn isolated_accounts_credentials_drafts_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [42; 32]).await.unwrap();
    let alice = store
        .user("https://id.example.invalid", "alice")
        .await
        .unwrap();
    let bob = store
        .user("https://id.example.invalid", "bob")
        .await
        .unwrap();
    assert_eq!(
        alice,
        store
            .user("https://id.example.invalid", "alice")
            .await
            .unwrap()
    );
    let a = store.connect(&alice, &account()).await.unwrap();
    assert!(store.accounts(&bob).await.unwrap().is_empty());
    assert!(matches!(
        store.authorize(&bob, &a.id).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.authorize(&alice, "../../control").await,
        Err(Error::NotFound)
    ));
    let scope = store.authorize(&alice, &a.id).await.unwrap();
    assert_eq!(
        store.credential(&scope).await.unwrap().password,
        account().password
    );
    let ciphertext: Vec<u8> = sqlx::query_scalar("SELECT credential FROM accounts")
        .fetch_one(store.control())
        .await
        .unwrap();
    assert!(!ciphertext
        .windows(account().password.len())
        .any(|w| w == account().password.as_bytes()));
    let d = store.save_draft(&scope, draft(&a.id)).await.unwrap();
    assert_eq!(d.revision, 1);
    assert!(matches!(
        store.save_draft(&scope, draft(&a.id)).await,
        Err(Error::Conflict)
    ));
    let wrong = Store::open(dir.path(), [99; 32]).await.unwrap();
    assert!(wrong.draft(&scope, &d.id).await.is_err());
    let wrong_scope = wrong.authorize(&alice, &a.id).await.unwrap();
    assert!(matches!(
        wrong.credential(&wrong_scope).await,
        Err(Error::Crypto)
    ));
    drop(store);
    let restarted = Store::open(dir.path(), [42; 32]).await.unwrap();
    let scope = restarted.authorize(&alice, &a.id).await.unwrap();
    assert_eq!(restarted.draft(&scope, &d.id).await.unwrap(), d);
    assert_eq!(restarted.events(&scope, 0).await.unwrap().len(), 1);
}
#[tokio::test]
async fn immutable_idempotent_submission_and_uncertain_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [42; 32]).await.unwrap();
    let user = store.user("issuer", "subject").await.unwrap();
    let a = store.connect(&user, &account()).await.unwrap();
    let scope = store.authorize(&user, &a.id).await.unwrap();
    let mut d = store.save_draft(&scope, draft(&a.id)).await.unwrap();
    let r = SubmitRequest {
        operation_id: "op-1".into(),
        draft_id: d.id.clone(),
        expected_revision: 1,
        due_at: None,
        timezone: "Europe/Tallinn".into(),
    };
    let submission = store.submit(&scope, &r).await.unwrap();
    assert_eq!(submission, store.submit(&scope, &r).await.unwrap());
    let mut other = r.clone();
    other.timezone = "UTC".into();
    assert!(matches!(
        store.submit(&scope, &other).await,
        Err(Error::Conflict)
    ));
    d.text = "later edit".into();
    store.save_draft(&scope, d).await.unwrap();
    assert_eq!(
        store
            .submission(&scope, &submission.id)
            .await
            .unwrap()
            .draft_revision,
        1
    );
    store
        .transition(
            &scope,
            &submission.id,
            SubmissionState::Scheduled,
            SubmissionState::Claimed,
        )
        .await
        .unwrap();
    store
        .transition(
            &scope,
            &submission.id,
            SubmissionState::Claimed,
            SubmissionState::Submitting,
        )
        .await
        .unwrap();
    assert!(store
        .transition(
            &scope,
            &submission.id,
            SubmissionState::Submitting,
            SubmissionState::Cancelled
        )
        .await
        .is_err());
    assert_eq!(
        store
            .recover_expired(&scope, "2099-01-01T00:00:00.000Z")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .submission(&scope, &submission.id)
            .await
            .unwrap()
            .state,
        SubmissionState::DeliveryUnknown
    );
    assert!(store
        .transition(
            &scope,
            &submission.id,
            SubmissionState::DeliveryUnknown,
            SubmissionState::Scheduled
        )
        .await
        .is_err());
    assert_eq!(store.events(&scope, 0).await.unwrap().len(), 6);
}
#[tokio::test]
async fn cancel_before_submission_is_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(dir.path(), [42; 32]).await.unwrap();
    let u = s.user("issuer", "subject").await.unwrap();
    let a = s.connect(&u, &account()).await.unwrap();
    let a = s.authorize(&u, &a.id).await.unwrap();
    s.save_draft(&a, draft(a.id())).await.unwrap();
    let r = SubmitRequest {
        operation_id: "op".into(),
        draft_id: "draft-1".into(),
        expected_revision: 1,
        due_at: Some("2099-01-01T00:00:00Z".into()),
        timezone: "UTC".into(),
    };
    let job = s.submit(&a, &r).await.unwrap();
    s.transition(
        &a,
        &job.id,
        SubmissionState::Scheduled,
        SubmissionState::Cancelled,
    )
    .await
    .unwrap();
    assert!(matches!(
        s.transition(
            &a,
            &job.id,
            SubmissionState::Scheduled,
            SubmissionState::Claimed
        )
        .await,
        Err(Error::Conflict)
    ));
}
