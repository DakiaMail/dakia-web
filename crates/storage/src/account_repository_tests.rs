use super::*;
#[path = "account_archive_contract.rs"]
mod archive;
#[path = "account_cleanup_contract.rs"]
mod cleanup;
#[path = "account_graph_contract.rs"]
mod graph;
#[path = "account_search_contract.rs"]
mod search;
#[path = "account_workers_contract.rs"]
mod workers;

fn draft(account: &str) -> Draft {
    serde_json::from_value(serde_json::json!({
        "id":"shared-draft","account_id":account,"revision":0,
        "to":[{"email":"recipient@example.invalid","name":null}],"cc":[],"bcc":[],
        "subject":"Fixture","text":"Original body","html":"","in_reply_to":null,
        "references":[],"attachments":[],"saved_at":""
    }))
    .unwrap()
}

async fn contract(a: AccountRepository, b: AccountRepository) {
    a.initialize_account().await.unwrap();
    b.initialize_account().await.unwrap();
    assert!(!a.removal_busy().await.unwrap());
    assert!(!b.removal_busy().await.unwrap());
    assert_eq!(a.event_cursor().await.unwrap(), 0);
    assert!(matches!(
        a.draft("shared-draft").await,
        Err(Error::NotFound)
    ));
    let mut saves = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let a = a.clone();
        saves.spawn(async move { a.save_draft(draft(&a.account)).await });
    }
    let mut saved = None;
    while let Some(result) = saves.join_next().await {
        match result.unwrap() {
            Ok(value) => assert!(saved.replace(value).is_none()),
            Err(Error::Conflict) => {}
            Err(error) => panic!("unexpected save failure: {error:?}"),
        }
    }
    let mut saved = saved.expect("one initial save");
    assert_eq!(saved.revision, 1);
    assert_eq!(a.event_cursor().await.unwrap(), 1);
    assert!(matches!(b.draft(&saved.id).await, Err(Error::NotFound)));
    let other = b.save_draft(draft(&b.account)).await.unwrap();
    assert_eq!(other.revision, 1);
    assert_eq!(b.event_cursor().await.unwrap(), 1);
    assert!(matches!(a.save_draft(other).await, Err(Error::Invalid)));
    let mut invalid = saved.clone();
    invalid.attachments.push(Attachment {
        id: "missing".into(),
        filename: "fixture.txt".into(),
        content_type: "text/plain".into(),
        size_bytes: 1,
    });
    assert!(matches!(a.save_draft(invalid).await, Err(Error::Invalid)));
    assert_eq!(a.event_cursor().await.unwrap(), 1);
    assert_eq!(a.draft(&saved.id).await.unwrap().revision, 1);

    let request = SubmitRequest {
        operation_id: "operation".into(),
        draft_id: saved.id.clone(),
        expected_revision: 1,
        due_at: None,
        timezone: "UTC".into(),
    };
    let mut submits = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let (a, request) = (a.clone(), request.clone());
        submits.spawn(async move {
            a.submit(&request, "2026-11-10T00:00:00.000Z")
                .await
                .unwrap()
        });
    }
    let mut scheduled = None;
    while let Some(result) = submits.join_next().await {
        let value = result.unwrap();
        if let Some(previous) = &scheduled {
            assert_eq!(&value, previous);
        } else {
            scheduled = Some(value);
        }
    }
    let scheduled = scheduled.unwrap();
    assert!(a.removal_busy().await.unwrap());
    assert!(!b.removal_busy().await.unwrap());
    assert_eq!(a.event_cursor().await.unwrap(), 2);
    let mut conflict = request.clone();
    conflict.timezone = "Europe/Tallinn".into();
    assert!(matches!(
        a.submit(&conflict, "2026-11-10T00:00:00.000Z").await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        b.submission(&scheduled.id).await,
        Err(Error::NotFound)
    ));
    // Identical idempotency keys in separate accounts are independent.
    let scheduled_b = b
        .submit(&request, "2026-11-10T00:00:00.000Z")
        .await
        .unwrap();
    assert_ne!(scheduled_b.id, scheduled.id);
    saved.text = "Edited after scheduling".into();
    a.save_draft(saved.clone()).await.unwrap();
    assert_eq!(
        a.submission_snapshot(&scheduled.id).await.unwrap().text,
        "Original body"
    );
    assert_eq!(
        a.due_submission("2026-11-10T00:00:00.000Z").await.unwrap(),
        Some(scheduled.id.clone())
    );

    let mut claims = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let (a, id) = (a.clone(), scheduled.id.clone());
        claims.spawn(async move {
            a.transition(&id, SubmissionState::Scheduled, SubmissionState::Claimed)
                .await
        });
    }
    let mut claimed = 0;
    while let Some(result) = claims.join_next().await {
        match result.unwrap() {
            Ok(_) => claimed += 1,
            Err(Error::Conflict) => {}
            Err(error) => panic!("claim: {error:?}"),
        }
    }
    assert_eq!(claimed, 1);
    assert!(a
        .due_submission("9999-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        a.expired_submissions("9999-01-01T00:00:00.000Z")
            .await
            .unwrap(),
        [(scheduled.id.clone(), "claimed".into())]
    );
    a.transition(
        &scheduled.id,
        SubmissionState::Claimed,
        SubmissionState::Submitting,
    )
    .await
    .unwrap();
    assert!(matches!(
        a.transition(
            &scheduled.id,
            SubmissionState::Submitting,
            SubmissionState::Scheduled
        )
        .await,
        Err(Error::Invalid)
    ));
    let first = PersistedMime {
        raw: b"first bytes\0".to_vec(),
        recipients: vec!["recipient@example.invalid".into()],
        message_id: "<fixture@example.invalid>".into(),
    };
    a.persist_mime(&scheduled.id, first.clone()).await.unwrap();
    a.persist_mime(
        &scheduled.id,
        PersistedMime {
            raw: b"replacement".to_vec(),
            ..first.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(a.mime(&scheduled.id).await.unwrap().unwrap().raw, first.raw);
    assert!(b.mime(&scheduled.id).await.unwrap().is_none());
    a.transition(
        &scheduled.id,
        SubmissionState::Submitting,
        SubmissionState::Accepted,
    )
    .await
    .unwrap();
    assert_eq!(a.needs_filing().await.unwrap().len(), 1);
    a.set_filing(&scheduled.id, FilingStatus::Filed)
        .await
        .unwrap();
    assert!(a.needs_filing().await.unwrap().is_empty());
    assert!(matches!(
        b.set_filing(&scheduled.id, FilingStatus::Filed).await,
        Err(Error::Conflict)
    ));

    a.put_provider_document("same-key", "a").await.unwrap();
    b.put_provider_document("same-key", "b").await.unwrap();
    assert_eq!(
        a.provider_document("same-key").await.unwrap().as_deref(),
        Some("a")
    );
    assert_eq!(
        b.provider_document("same-key").await.unwrap().as_deref(),
        Some("b")
    );
    for (key, value) in [
        ("member-reference:message:inbox", "one"),
        ("member-reference:message:sent", "two"),
        ("member-reference:messages:inbox", "outside"),
    ] {
        a.put_provider_document(key, value).await.unwrap();
    }
    assert_eq!(
        a.message_reference_documents("message").await.unwrap(),
        ["one", "two"]
    );
    let mut attempts = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let a = a.clone();
        attempts.spawn(async move { a.claim_unsubscribe("http-unsubscribe").await.unwrap() });
    }
    let mut attempts_won = 0;
    while let Some(result) = attempts.join_next().await {
        attempts_won += usize::from(result.unwrap());
    }
    assert_eq!(attempts_won, 1);
    let cursor = a.event_cursor().await.unwrap();
    // The provider marker is written first, but a submission uniqueness failure
    // must roll it back along with all events and outbox work.
    assert!(a
        .enqueue_unsubscribe("operation", draft(&a.account))
        .await
        .is_err());
    assert!(a.provider_document("operation").await.unwrap().is_none());
    assert_eq!(a.event_cursor().await.unwrap(), cursor);
    a.enqueue_unsubscribe("mail-unsubscribe", draft(&a.account))
        .await
        .unwrap();
    let marker = a
        .provider_document("mail-unsubscribe")
        .await
        .unwrap()
        .unwrap();
    a.enqueue_unsubscribe("mail-unsubscribe", draft(&a.account))
        .await
        .unwrap();
    assert_eq!(
        a.provider_document("mail-unsubscribe")
            .await
            .unwrap()
            .unwrap(),
        marker
    );
    assert_eq!(a.event_cursor().await.unwrap(), cursor + 1);
    let events = a.events(0).await.unwrap();
    assert_eq!(
        events.iter().map(|e| e.cursor).collect::<Vec<_>>(),
        (1..=cursor + 1).collect::<Vec<_>>()
    );
    assert!(events.iter().all(|e| e.account_id == a.account));
    assert_eq!(a.events(cursor).await.unwrap().len(), 1);
    assert_eq!(b.event_cursor().await.unwrap(), 2);

    // Opaque unsubscribe receipts coexist with JSON requests in this view.
    assert!(a.scheduled_messages().await.unwrap().is_empty());
    let mut delayed = request;
    delayed.operation_id = "delayed".into();
    delayed.expected_revision = a.draft("shared-draft").await.unwrap().revision;
    delayed.due_at = Some("2026-12-01T00:00:00.000Z".into());
    let delayed_submission = a
        .submit(&delayed, delayed.due_at.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(
        a.scheduled_messages().await.unwrap()[0].submission.id,
        delayed_submission.id
    );
    assert!(b.scheduled_messages().await.unwrap().is_empty());
    assert_eq!(a.drafts().await.unwrap().len(), 1);
    assert_eq!(a.submissions().await.unwrap().len(), 3);
    attachment_contract(&a, &b).await;
    mailbox_contract(&a, &b).await;
    scan_contract(&a, &b).await;
    publication_contract(&a).await;
    mutation_worker_contract(&a, &b).await;
    workers::contract(&a, &b).await;
    graph::contract(&a, &b).await;
    cleanup::contract(&a, &b).await;
    search::contract(&a, &b).await;
    merged_main_contract(&a, &b).await;
    graph_delta_contract(&a, &b).await;
    archive::contract(&a).await;
}

async fn make_mutation_due(repository: &AccountRepository, id: &str) {
    match &repository.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE outbox SET lease_until='2000-01-01T00:00:00.000Z' WHERE target_id=? AND kind='mutation'").bind(id).execute(db).await.unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE outbox SET lease_until='2000-01-01T00:00:00.000Z' WHERE account_id=$1 AND target_id=$2 AND kind='mutation'").bind(&repository.account).bind(id).execute(db).await.unwrap();
        }
    }
}
async fn mutation_worker_contract(a: &AccountRepository, b: &AccountRepository) {
    for _ in 0..20 {
        let Some(mut operation) = a.claim_mutation(&["rule".into()]).await.unwrap() else {
            break;
        };
        operation.status = OperationStatus::Failed;
        a.finish_mutation(operation, None).await.unwrap();
    }
    assert!(a.claim_mutation(&[]).await.unwrap().is_none());
    let rows = a
        .ingest_messages(
            vec![
                message(&a.account, "ordered"),
                message(&a.account, "parallel"),
            ],
            &[],
            &[],
            None,
        )
        .await
        .unwrap();
    let mut ordered = rows[0].clone();
    a.operation(mutation(
        &ordered,
        "worker-one",
        Change::Star { value: true },
    ))
    .await
    .unwrap();
    ordered = a.message(&ordered.id).await.unwrap();
    a.operation(mutation(
        &ordered,
        "worker-two",
        Change::Read { value: true },
    ))
    .await
    .unwrap();
    a.operation(mutation(
        &rows[1],
        "worker-other",
        Change::Star { value: true },
    ))
    .await
    .unwrap();
    let mut workers = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let a = a.clone();
        workers.spawn(async move { a.claim_mutation(&[]).await.unwrap() });
    }
    let mut claimed = std::collections::BTreeMap::new();
    while let Some(result) = workers.join_next().await {
        if let Some(op) = result.unwrap() {
            assert!(claimed.insert(op.id.clone(), op).is_none());
        }
    }
    assert_eq!(
        claimed.keys().cloned().collect::<Vec<_>>(),
        ["worker-one", "worker-other"]
    );
    let one = claimed.remove("worker-one").unwrap();
    assert!(matches!(
        b.finish_mutation(one.clone(), None).await,
        Err(Error::Invalid)
    ));
    assert!(a.retry_mutation(&one.id).await.unwrap());
    let mut other = claimed.remove("worker-other").unwrap();
    other.status = OperationStatus::Applied;
    a.finish_mutation(other, Some(vec!["Archive".into()]))
        .await
        .unwrap();
    assert_eq!(a.message("parallel").await.unwrap().folder_ids, ["Archive"]);
    assert!(a.claim_mutation(&[]).await.unwrap().is_none());
    for remaining in [true, true, false] {
        make_mutation_due(a, "worker-one").await;
        let mut operation = a.claim_mutation(&[]).await.unwrap().unwrap();
        assert_eq!(operation.id, "worker-one");
        assert_eq!(a.retry_mutation(&operation.id).await.unwrap(), remaining);
        if !remaining {
            operation.status = OperationStatus::Failed;
            a.finish_mutation(operation, None).await.unwrap();
        }
    }
    let operation = a.claim_mutation(&[]).await.unwrap().unwrap();
    assert_eq!(operation.id, "worker-two");
    // A lost claimed move or flag operation is terminal, never blindly replayed.
    a.recover_mutations().await.unwrap();
    assert_eq!(
        a.operation_status("worker-two").await.unwrap().status,
        OperationStatus::Failed
    );
    assert!(a.claim_mutation(&[]).await.unwrap().is_none());
    assert!(a
        .provider_document("mutation-retry:worker-one")
        .await
        .unwrap()
        .is_none());
    for (id, running) in [
        ("disabled-before-claim", false),
        ("disabled-after-claim", true),
    ] {
        let rule:MailRule=serde_json::from_value(serde_json::json!({"id":id,"preset":"block_sender","enabled":true,"activated_at":"2026-11-10T00:00:00Z","scope":"all_mailboxes","condition":{"kind":"sender_is","email":"sender@example.invalid"},"actions":["trash","suppress_notifications"]})).unwrap();
        a.ingest_messages(
            vec![message(&a.account, id)],
            &[],
            std::slice::from_ref(&rule),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            a.rule_activity()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.rule_id == id)
                .unwrap()
                .pending,
            1
        );
        assert!(b
            .rule_activity()
            .await
            .unwrap()
            .iter()
            .all(|r| r.rule_id != id));
        assert_eq!(
            a.rule_executions()
                .await
                .unwrap()
                .into_iter()
                .find(|r| r.rule_id == id)
                .unwrap()
                .state,
            "pending"
        );
        if running {
            a.claim_mutation(&[id.into()]).await.unwrap().unwrap();
        }
        let mut disabled = rule.clone();
        disabled.enabled = false;
        a.rule_changed(&disabled).await.unwrap();
        assert!(a.claim_mutation(&[]).await.unwrap().is_none());
        assert_eq!(
            a.operation_status(&format!("rule:{id}:{id}"))
                .await
                .unwrap()
                .status,
            if running {
                OperationStatus::Pending
            } else {
                OperationStatus::Failed
            }
        );
        if !running {
            assert!(a
                .rule_executions()
                .await
                .unwrap()
                .iter()
                .all(|r| r.rule_id != id));
        }
        a.recover_mutations().await.unwrap();
    }
    a.link_identity("shared-provider-reference", "ordered")
        .await
        .unwrap();
    a.link_identity("shared-provider-reference", "parallel")
        .await
        .unwrap();
    b.link_identity("shared-provider-reference", "shared-message")
        .await
        .unwrap();
}

async fn cached_search_document(repository: &AccountRepository, message: &Message) {
    match &repository.backend {
        Backend::Sqlite(db) => {
            sqlx::query("INSERT INTO search_documents(message_id,source,sender,recipients,subject,received_at,body,body_state) VALUES(?,?, 'sender','','fixture',?,'body','ready')").bind(&message.id).bind(&message.provider_id).bind(&message.received_at).execute(db).await.unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("INSERT INTO search_documents(account_id,message_id,source,sender,recipients,subject,received_at,body,body_state) VALUES($1,$2,$3,'sender','','fixture',$4,'body','ready')").bind(&repository.account).bind(&message.id).bind(&message.provider_id).bind(&message.received_at).execute(db).await.unwrap();
        }
    }
}
async fn cached_search_exists(repository: &AccountRepository, id: &str) -> bool {
    match &repository.backend {
        Backend::Sqlite(db) => {
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM search_documents WHERE message_id=?)")
                .bind(id)
                .fetch_one(db)
                .await
                .unwrap()
        }
        Backend::Postgres(db) => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM search_documents WHERE account_id=$1 AND message_id=$2)",
        )
        .bind(&repository.account)
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap(),
    }
}
async fn publication_contract(a: &AccountRepository) {
    let m = message(&a.account, "publication");
    let m = a
        .ingest_messages(vec![m], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    cached_search_document(a, &m).await;
    let first = receipts(a)
        .await
        .into_iter()
        .find(|r| r.0 == m.id)
        .unwrap()
        .1;
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query(
                "UPDATE search_documents SET body='updated',body_state='ready' WHERE message_id=?",
            )
            .bind(&m.id)
            .execute(db)
            .await
            .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE search_documents SET body='updated',body_state='ready' WHERE account_id=$1 AND message_id=$2").bind(&a.account).bind(&m.id).execute(db).await.unwrap();
        }
    }
    let latest = receipts(a)
        .await
        .into_iter()
        .find(|r| r.0 == m.id)
        .unwrap()
        .1;
    assert!(latest > first);
    let changed = match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("DELETE FROM search_index_outbox WHERE message_id=? AND sequence=?")
                .bind(&m.id)
                .bind(first)
                .execute(db)
                .await
                .unwrap()
                .rows_affected()
        }
        Backend::Postgres(db) => sqlx::query(
            "DELETE FROM search_index_outbox WHERE account_id=$1 AND message_id=$2 AND sequence=$3",
        )
        .bind(&a.account)
        .bind(&m.id)
        .bind(first)
        .execute(db)
        .await
        .unwrap()
        .rows_affected(),
    };
    assert_eq!(
        changed, 0,
        "an old acknowledgement must not discard a newer change"
    );
    let mut removed = m.clone();
    removed.folder_ids.clear();
    a.ingest_messages(vec![removed], &[], &[], None)
        .await
        .unwrap();
    assert!(!cached_search_exists(a, &m.id).await);
    assert!(
        receipts(a)
            .await
            .into_iter()
            .find(|r| r.0 == m.id)
            .unwrap()
            .1
            > latest
    );

    let mut epoch = message(&a.account, "epoch-publication");
    epoch.folder_ids = vec!["Epoch".into()];
    epoch.provider_id = serde_json::json!({"folder":"Epoch","uidvalidity":10,"uid":1}).to_string();
    let epoch = a
        .ingest_messages(vec![epoch], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    a.put_provider_document("thread:epoch:Epoch", "10")
        .await
        .unwrap();
    cached_search_document(a, &epoch).await;
    let before = receipts(a)
        .await
        .into_iter()
        .find(|r| r.0 == epoch.id)
        .unwrap()
        .1;
    a.put_provider_document("thread:epoch:Epoch", "11")
        .await
        .unwrap();
    assert!(!cached_search_exists(a, &epoch.id).await);
    assert!(
        receipts(a)
            .await
            .into_iter()
            .find(|r| r.0 == epoch.id)
            .unwrap()
            .1
            > before
    );
}

fn page(
    account: &str,
    rows: Vec<Message>,
    continuation: Option<&str>,
    coverage: CoverageState,
) -> SearchPage {
    SearchPage {
        cache_preview: None,
        index_page: None,
        search: None,
        supplement: None,
        rows,
        continuation: continuation.map(str::to_owned),
        coverage: vec![AccountCoverage {
            account_id: account.into(),
            folder_ids: vec!["History".into()],
            inbox: None,
            state: coverage,
            error: None,
        }],
        classification: ClassificationCoverage {
            classified: 0,
            total: 0,
        },
        pending: false,
        counts_exact: false,
    }
}
async fn push_jobs(repository: &AccountRepository) -> Vec<(String, String)> {
    match &repository.backend {
        Backend::Sqlite(db) => {
            sqlx::query_as("SELECT message_id,state FROM push_jobs ORDER BY message_id")
                .fetch_all(db)
                .await
                .unwrap()
        }
        Backend::Postgres(db) => sqlx::query_as(
            "SELECT message_id,state FROM push_jobs WHERE account_id=$1 ORDER BY message_id",
        )
        .bind(&repository.account)
        .fetch_all(db)
        .await
        .unwrap(),
    }
}
async fn expire_scan(repository: &AccountRepository, folder: &str) {
    match &repository.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE folder_scans SET last_scanned_at=0 WHERE folder=?")
                .bind(folder)
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query(
                "UPDATE folder_scans SET last_scanned_at=0 WHERE account_id=$1 AND folder=$2",
            )
            .bind(&repository.account)
            .bind(folder)
            .execute(db)
            .await
            .unwrap();
        }
    }
}
async fn scan_contract(a: &AccountRepository, b: &AccountRepository) {
    let mut old = message(&a.account, "scan-old");
    old.folder_ids = vec!["History".into()];
    let mut keep = message(&a.account, "scan-keep");
    keep.folder_ids = vec!["History".into()];
    a.ingest_messages(vec![old.clone(), keep.clone()], &[], &[], None)
        .await
        .unwrap();
    a.put_provider_document("thread:epoch:History", "100")
        .await
        .unwrap();
    let (generation, _) = a.scan_state("History").await.unwrap().unwrap();
    assert!(b.scan_generation("History").await.unwrap().is_none());
    a.record_scan(
        "History",
        generation,
        &page(
            &a.account,
            vec![keep.clone()],
            Some("next"),
            CoverageState::Complete,
        ),
        &[],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        a.scan_state("History").await.unwrap(),
        Some((generation, Some("next".into())))
    );
    assert_eq!(a.message(&old.id).await.unwrap().folder_ids, ["History"]);
    let mut late = message(&a.account, "scan-late");
    late.folder_ids = vec!["History".into()];
    a.ingest_messages(vec![late.clone()], &[], &[], None)
        .await
        .unwrap();
    a.record_scan(
        "History",
        generation,
        &page(&a.account, vec![], None, CoverageState::Complete),
        &[],
        &[],
    )
    .await
    .unwrap();
    assert!(a.message(&old.id).await.unwrap().folder_ids.is_empty());
    assert_eq!(a.message(&keep.id).await.unwrap().folder_ids, ["History"]);
    assert_eq!(a.message(&late.id).await.unwrap().folder_ids, ["History"]);
    assert!(a.scan_state("History").await.unwrap().is_none());
    assert!(a
        .terminal_scan_folders()
        .await
        .unwrap()
        .contains(&"History".into()));
    assert!(matches!(
        a.record_scan(
            "History",
            generation,
            &page(&a.account, vec![], None, CoverageState::Complete),
            &[],
            &[]
        )
        .await,
        Err(Error::Conflict)
    ));
    a.confirm_unchanged_scan("History", generation)
        .await
        .unwrap();
    assert!(a.verified_snapshot_current("History").await.unwrap());
    // A late foreground projection requires reconciliation before refreshing
    // the old completed proof. It was not part of that traversal's deletion set.
    expire_scan(a, "History").await;
    assert!(a.completed_scan_due("History").await.unwrap().is_none());
    let repaired = a.scan_generation("History").await.unwrap().unwrap();
    assert_eq!(repaired, generation + 1);
    assert!(!a.verified_snapshot_current("History").await.unwrap());
    let mut foreign = keep.clone();
    foreign.account_id = b.account.clone();
    let before = a.event_cursor().await.unwrap();
    assert!(matches!(
        a.record_scan(
            "History",
            repaired,
            &page(&a.account, vec![foreign], None, CoverageState::Complete),
            &[],
            &[]
        )
        .await,
        Err(Error::Invalid)
    ));
    assert_eq!(a.event_cursor().await.unwrap(), before);
    assert_eq!(
        a.scan_state("History").await.unwrap(),
        Some((repaired, None))
    );
    a.record_scan(
        "History",
        repaired,
        &page(
            &a.account,
            vec![keep.clone(), late],
            None,
            CoverageState::Complete,
        ),
        &[],
        &[],
    )
    .await
    .unwrap();
    a.confirm_unchanged_scan("History", repaired).await.unwrap();
    a.put_provider_document("reference:scan-keep", &keep.provider_id)
        .await
        .unwrap();
    assert!(!a
        .invalidate_stale_reference(&keep.id, "History", "wrong")
        .await
        .unwrap());
    assert!(a.verified_snapshot_current("History").await.unwrap());
    assert!(a
        .invalidate_stale_reference(&keep.id, "History", &keep.provider_id)
        .await
        .unwrap());
    assert!(!a
        .invalidate_stale_reference(&keep.id, "History", &keep.provider_id)
        .await
        .unwrap());
    assert!(!a.verified_snapshot_current("History").await.unwrap());
    assert_eq!(
        a.scan_generation("History").await.unwrap(),
        Some(repaired + 1)
    );

    let mut limited = message(&a.account, "limited-old");
    limited.folder_ids = vec!["Limited".into()];
    a.ingest_messages(vec![limited.clone()], &[], &[], None)
        .await
        .unwrap();
    let (limited_gen, _) = a.scan_state("Limited").await.unwrap().unwrap();
    a.record_scan(
        "Limited",
        limited_gen,
        &page(&a.account, vec![], None, CoverageState::Partial),
        &[],
        &[],
    )
    .await
    .unwrap();
    assert!(a.scan_state("Limited").await.unwrap().is_none());
    assert_eq!(
        a.message(&limited.id).await.unwrap().folder_ids,
        ["Limited"]
    );
    a.reset_scan("Limited").await.unwrap();
    let (next_gen, _) = a.scan_state("Limited").await.unwrap().unwrap();
    assert_eq!(next_gen, limited_gen + 1);
    a.record_scan(
        "Limited",
        next_gen,
        &page(&a.account, vec![], None, CoverageState::Complete),
        &[],
        &[],
    )
    .await
    .unwrap();
    assert!(a.message(&limited.id).await.unwrap().folder_ids.is_empty());
    expire_scan(a, "Limited").await;
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let a = a.clone();
        readers.spawn(async move { a.scan_state("Limited").await.unwrap().unwrap().0 });
    }
    while let Some(result) = readers.join_next().await {
        assert_eq!(result.unwrap(), next_gen + 1);
    }

    assert_eq!(
        b.inbox_coverage().await.unwrap().state,
        CoverageState::Pending
    );
    let existing = b.message("shared-message").await.unwrap();
    b.seed_inbox_memberships().await.unwrap();
    let (inbox_gen, _) = b.scan_state("INBOX").await.unwrap().unwrap();
    let subscriptions = vec![serde_json::json!({"id":"fixture-push"})];
    b.record_scan(
        "INBOX",
        inbox_gen,
        &page(&b.account, vec![existing], None, CoverageState::Complete),
        &subscriptions,
        &[],
    )
    .await
    .unwrap();
    assert!(push_jobs(b).await.is_empty());
    assert_eq!(
        b.inbox_coverage().await.unwrap().state,
        CoverageState::Complete
    );
    let arrival = message(&b.account, "arrival");
    b.ingest_messages(vec![arrival.clone()], &[], &[], None)
        .await
        .unwrap();
    for _ in 0..3 {
        b.record_incremental(
            "INBOX",
            std::slice::from_ref(&arrival),
            "head-checkpoint",
            &subscriptions,
            &[],
        )
        .await
        .unwrap();
    }
    assert_eq!(push_jobs(b).await, [("arrival".into(), "pending".into())]);
    assert_eq!(
        b.provider_document("sync:head:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("head-checkpoint")
    );
    let failed = message(&b.account, "failed-arrival");
    b.ingest_messages(vec![failed.clone()], &[], &[], None)
        .await
        .unwrap();
    assert!(matches!(
        b.record_incremental(
            "INBOX",
            std::slice::from_ref(&failed),
            "bad-checkpoint",
            &[serde_json::json!({})],
            &[]
        )
        .await,
        Err(Error::Invalid)
    ));
    assert_eq!(
        b.provider_document("sync:head:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("head-checkpoint")
    );
    b.record_incremental(
        "INBOX",
        std::slice::from_ref(&failed),
        "recovered-checkpoint",
        &subscriptions,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(push_jobs(b).await.len(), 2);
    let rows = b
        .inbox_rows(None, None, &ClassificationFilter::default())
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    let next = b
        .inbox_rows(
            Some((&rows[0].received_at, &rows[0].id)),
            None,
            &ClassificationFilter::default(),
        )
        .await
        .unwrap();
    assert_eq!(next.len(), 2);
    assert!(next.iter().all(|m| m.id != rows[0].id));
    b.put_provider_document("inbox:refresh-error", "fixture outage")
        .await
        .unwrap();
    assert_eq!(
        b.inbox_coverage().await.unwrap().state,
        CoverageState::Error
    );
    assert_eq!(
        b.inbox_rows(None, None, &ClassificationFilter::default())
            .await
            .unwrap()
            .len(),
        3
    );
    b.clear_provider_document("inbox:refresh-error")
        .await
        .unwrap();
    assert_eq!(
        b.inbox_coverage().await.unwrap().state,
        CoverageState::Complete
    );
}

fn message(account: &str, id: &str) -> Message {
    serde_json::from_value(serde_json::json!({"id":id,"account_id":account,"thread_id":id,"provider_id":format!("provider:{id}"),"folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":"Fixture"},"to":[],"subject":"Synthetic mailbox","preview":"Preview","received_at":"2030-11-10T00:00:00.000Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap()
}
fn mutation(message: &Message, id: &str, change: Change) -> Operation {
    Operation {
        id: id.into(),
        account_id: message.account_id.clone(),
        target_id: message.id.clone(),
        expected_revision: message.revision,
        change,
        status: OperationStatus::Pending,
        revision: 0,
    }
}
async fn receipts(repository: &AccountRepository) -> Vec<(String, i64)> {
    match &repository.backend {
        Backend::Sqlite(db)=>sqlx::query_as("SELECT message_id,sequence FROM search_index_outbox ORDER BY message_id").fetch_all(db).await.unwrap(),
        Backend::Postgres(db)=>sqlx::query_as("SELECT message_id,sequence FROM search_index_outbox WHERE account_id=$1 ORDER BY message_id").bind(&repository.account).fetch_all(db).await.unwrap(),
    }
}
async fn mailbox_contract(a: &AccountRepository, b: &AccountRepository) {
    let initial_cursor = a.event_cursor().await.unwrap();
    let mut fresh = message(&a.account, "shared-message");
    let mut current = a
        .ingest_messages(vec![fresh.clone()], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(current.revision, 1);
    assert!(matches!(b.message(&current.id).await, Err(Error::NotFound)));
    let original_receipts = receipts(a).await;
    for _ in 0..4 {
        assert_eq!(
            a.ingest_messages(vec![fresh.clone()], &[], &[], None)
                .await
                .unwrap()[0],
            current
        );
    }
    assert_eq!(a.event_cursor().await.unwrap(), initial_cursor + 1);
    assert_eq!(receipts(a).await, original_receipts);
    let read = mutation(&current, "read-message", Change::Read { value: true });
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let (a, read) = (a.clone(), read.clone());
        tasks.spawn(async move { a.operation(read).await.unwrap() });
    }
    let mut applied = None;
    while let Some(result) = tasks.join_next().await {
        let op = result.unwrap();
        if let Some(prior) = &applied {
            assert_eq!(&op, prior);
        } else {
            applied = Some(op);
        }
    }
    assert_eq!(
        a.operation_status("read-message").await.unwrap().revision,
        2
    );
    let mut conflicting = read.clone();
    conflicting.change = Change::Star { value: true };
    assert!(matches!(
        a.operation(conflicting).await,
        Err(Error::Conflict)
    ));
    current = a.message(&current.id).await.unwrap();
    assert!(!current.unread);
    // A provider refresh must retain pending local flags and manual categories.
    current = a
        .ingest_messages(vec![fresh.clone()], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    assert!(!current.unread);
    assert_eq!(current.revision, 2);
    a.operation(mutation(
        &current,
        "category",
        Change::Category {
            value: "people".into(),
        },
    ))
    .await
    .unwrap();
    fresh.category = Some("newsletters".into());
    fresh.category_status = CategoryStatus::Ready;
    current = a
        .ingest_messages(vec![fresh.clone()], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(current.category.as_deref(), Some("people"));
    let before = a.event_cursor().await.unwrap();
    assert!(matches!(
        a.operation(mutation(
            &current,
            "invalid-category",
            Change::Category {
                value: "invalid".into()
            }
        ))
        .await,
        Err(Error::Invalid)
    ));
    assert_eq!(a.event_cursor().await.unwrap(), before);
    assert_eq!(a.message(&current.id).await.unwrap(), current);
    assert!(matches!(
        b.operation(mutation(&current, "forged", Change::Trash))
            .await,
        Err(Error::Invalid)
    ));
    a.operation(mutation(&current, "archive", Change::Archive))
        .await
        .unwrap();
    let stored = a.message(&current.id).await.unwrap();
    assert_eq!(stored.folder_ids, ["INBOX"]);
    assert!(a
        .visible_message(stored.clone())
        .await
        .unwrap()
        .folder_ids
        .is_empty());
    a.operation(mutation(&stored, "trash", Change::Trash))
        .await
        .unwrap();
    assert_eq!(
        a.visible_message(a.message(&current.id).await.unwrap())
            .await
            .unwrap()
            .folder_ids,
        ["Trash"]
    );
    assert_eq!(a.message(&current.id).await.unwrap().folder_ids, ["INBOX"]);
    assert!(matches!(
        b.operation_status("read-message").await,
        Err(Error::NotFound)
    ));
    assert_eq!(b.catalogue().await.unwrap().len(), 0);
    b.ingest_messages(vec![message(&b.account, "shared-message")], &[], &[], None)
        .await
        .unwrap();
    assert!(b.message("shared-message").await.unwrap().unread);

    let rule:MailRule=serde_json::from_value(serde_json::json!({"id":"rule","preset":"block_sender","enabled":true,"activated_at":"2026-11-10T00:00:00Z","scope":"all_mailboxes","condition":{"kind":"sender_is","email":"sender@example.invalid"},"actions":["trash","suppress_notifications"]})).unwrap();
    let arriving = message(&a.account, "new-arrival");
    let indexed = a
        .ingest_messages(
            vec![arriving.clone()],
            &[],
            std::slice::from_ref(&rule),
            Some("2020-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
    assert_eq!(indexed[0].revision, 2);
    let operation_id = "rule:rule:new-arrival";
    assert_eq!(
        a.operation_status(operation_id).await.unwrap().change,
        Change::Trash
    );
    let before = a.event_cursor().await.unwrap();
    a.ingest_messages(
        vec![arriving],
        &[],
        std::slice::from_ref(&rule),
        Some("2020-01-01T00:00:00Z"),
    )
    .await
    .unwrap();
    assert_eq!(a.event_cursor().await.unwrap(), before);
    let mut excluded = message(&a.account, "excluded-arrival");
    excluded.folder_ids.push("Deleted Items".into());
    let documents = vec![(
        "probe".into(),
        serde_json::json!({"folders":[{"name":"Deleted Items","attributes":[format!("{}Trash",char::from(92))]}]}).to_string(),
    )];
    a.ingest_messages(
        vec![excluded],
        &documents,
        std::slice::from_ref(&rule),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        a.operation_status("rule:rule:excluded-arrival").await,
        Err(Error::NotFound)
    ));
    let mut gmail = message(&a.account, "gmail");
    gmail.provider_id = "gmail:fixture".into();
    let state = ContentReferences {
        token: "old-token".into(),
        source: gmail.provider_id.clone(),
        blocked: vec!["reference".into()],
        blocked_at: 100,
    };
    a.put_provider_document(
        "content:references:gmail",
        &serde_json::to_string(&state).unwrap(),
    )
    .await
    .unwrap();
    for (observation, blocked) in [(50, true), (101, false)] {
        a.ingest_messages(
            vec![gmail.clone()],
            &[
                ("reference:gmail".into(), "reference".into()),
                (
                    "__content_observation:gmail".into(),
                    observation.to_string(),
                ),
            ],
            &[],
            None,
        )
        .await
        .unwrap();
        let current: ContentReferences = serde_json::from_str(
            &a.provider_document("content:references:gmail")
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(!current.blocked.is_empty(), blocked);
        assert_eq!(current.token == "old-token", blocked);
    }
    a.put_provider_document("content:references:rollback", "invalid-json")
        .await
        .unwrap();
    let before = a.event_cursor().await.unwrap();
    let before_receipts = receipts(a).await;
    let mut failed = message(&a.account, "rollback");
    failed.provider_id = "gmail:rollback".into();
    assert!(a
        .ingest_messages(
            vec![failed],
            &[
                ("atomic-checkpoint".into(), "must-rollback".into()),
                ("reference:rollback".into(), "reference".into())
            ],
            &[],
            None
        )
        .await
        .is_err());
    assert!(matches!(a.message("rollback").await, Err(Error::NotFound)));
    assert!(a
        .provider_document("atomic-checkpoint")
        .await
        .unwrap()
        .is_none());
    assert_eq!(a.event_cursor().await.unwrap(), before);
    assert_eq!(receipts(a).await, before_receipts);
    assert!(a
        .catalogue()
        .await
        .unwrap()
        .iter()
        .all(|m| m.account_id == a.account));
    let mut writers = tokio::task::JoinSet::new();
    for n in 0..8 {
        let a = a.clone();
        writers.spawn(async move {
            let mut row = message(&a.account, "concurrent-message");
            row.subject = format!("Concurrent {n}");
            a.ingest_messages(vec![row], &[], &[], None).await.unwrap()[0].revision
        });
    }
    let mut revisions = std::collections::BTreeSet::new();
    while let Some(result) = writers.join_next().await {
        revisions.insert(result.unwrap());
    }
    assert_eq!(revisions, (1..=8).collect());
}

async fn attachment_contract(a: &AccountRepository, b: &AccountRepository) {
    let attachment = Attachment {
        id: Uuid::new_v4().to_string(),
        filename: "fixture.txt".into(),
        content_type: "text/plain".into(),
        size_bytes: 3,
    };
    a.put_attachment_metadata(&attachment).await.unwrap();
    assert!(a.attachment_retained(&attachment.id).await.unwrap());
    assert!(matches!(
        b.attachment_metadata(&attachment.id).await,
        Err(Error::NotFound)
    ));
    assert!(!b.attachment_retained(&attachment.id).await.unwrap());
    let mut value = draft(&b.account);
    value.id = "attachment-draft".into();
    value.attachments = vec![attachment.clone()];
    assert!(matches!(b.save_draft(value).await, Err(Error::Invalid)));
    let mut other = attachment.clone();
    other.size_bytes = 2;
    b.put_attachment_metadata(&other).await.unwrap();
    assert_eq!(
        a.attachment_metadata(&attachment.id)
            .await
            .unwrap()
            .size_bytes,
        3
    );
    let orphan = Attachment {
        id: Uuid::new_v4().to_string(),
        ..attachment.clone()
    };
    a.put_attachment_metadata(&orphan).await.unwrap();
    let mut value = draft(&a.account);
    value.id = "attachment-draft".into();
    value.attachments = vec![attachment.clone()];
    let mut value = a.save_draft(value).await.unwrap();
    a.submit(
        &SubmitRequest {
            operation_id: "attachment-send".into(),
            draft_id: value.id.clone(),
            expected_revision: value.revision,
            due_at: None,
            timezone: "UTC".into(),
        },
        "2026-11-10T00:00:00.000Z",
    )
    .await
    .unwrap();
    value.attachments.clear();
    a.save_draft(value).await.unwrap();
    assert_eq!(
        a.cleanup_attachment_metadata("9999-01-01T00:00:00.000Z")
            .await
            .unwrap(),
        std::slice::from_ref(&orphan.id)
    );
    assert!(matches!(
        a.attachment_metadata(&orphan.id).await,
        Err(Error::NotFound)
    ));
    assert_eq!(
        a.attachment_metadata(&attachment.id)
            .await
            .unwrap()
            .size_bytes,
        3
    );
    assert_eq!(
        b.cleanup_attachment_metadata("9999-01-01T00:00:00.000Z")
            .await
            .unwrap(),
        std::slice::from_ref(&attachment.id)
    );
    assert!(a.attachment_retained(&attachment.id).await.unwrap());
}

#[tokio::test]
async fn sqlite_account_submission_contract() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [19; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let input:ConnectAccount=serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account_a = store.connect(&user, &input).await.unwrap();
    let account_b = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account_a.id).await.unwrap();
    let b = store.authorize(&user, &account_b.id).await.unwrap();
    contract(
        store.account_repository(&a).await.unwrap(),
        store.account_repository(&b).await.unwrap(),
    )
    .await;
}

#[tokio::test]
async fn scheduled_projection_migrates_json_and_opaque_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let db = crate::pool(&dir.path().join("legacy.sqlite"))
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE submissions(id TEXT PRIMARY KEY,request TEXT NOT NULL,due_at TEXT NOT NULL)",
    )
    .execute(&db)
    .await
    .unwrap();
    for (id, request) in [
        ("scheduled", r#"{"due_at":"2026-12-01T00:00:00Z"}"#),
        ("immediate", r#"{"due_at":null}"#),
        ("absent", r#"{"operation_id":"fixture"}"#),
        ("opaque", "submission:fixture"),
        ("invalid", "{invalid"),
    ] {
        sqlx::query("INSERT INTO submissions VALUES(?,?,'2026-11-10T00:00:00Z')")
            .bind(id)
            .bind(request)
            .execute(&db)
            .await
            .unwrap();
    }
    sqlx::Executor::execute(
        &db,
        include_str!("../migrations/account/0031_scheduled_projection.sql"),
    )
    .await
    .unwrap();
    let scheduled: Vec<String> =
        sqlx::query_scalar("SELECT id FROM submissions WHERE scheduled_by_user=1")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(scheduled, ["scheduled"]);
}

#[tokio::test]
#[ignore = "requires disposable dakia_account_test database; DAKIA_TEST_ACCOUNT_POSTGRES_URL"]
async fn postgres_account_submission_contract() {
    let url = std::env::var("DAKIA_TEST_ACCOUNT_POSTGRES_URL").unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    assert_eq!(
        options.get_database(),
        Some("dakia_account_test"),
        "refuse to reset any other database"
    );
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for schema in ["dakia_mail", "dakia_control"] {
        sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
    let control = crate::control_repository::ControlRepository::open_postgres(&url)
        .await
        .unwrap();
    let owner = control.user("fixture", "owner").await.unwrap();
    let guest = control.user("fixture", "guest").await.unwrap();
    let mut accounts = vec![];
    for user in [&owner, &guest] {
        let account = Account {
            id: Uuid::new_v4().to_string(),
            email: "fixture@example.invalid".into(),
            display_name: "Fixture".into(),
            provider: ProviderKind::Imap,
            state: AccountState::Ready,
            revision: 1,
        };
        control
            .insert_account(user, &account, b"sealed-fixture", "2026-11-10T00:00:00Z")
            .await
            .unwrap();
        accounts.push(account);
    }
    let a = AccountRepository::postgres(&url, &owner, &accounts[0].id)
        .await
        .unwrap();
    let b = AccountRepository::postgres(&url, &guest, &accounts[1].id)
        .await
        .unwrap();
    contract(a.clone(), b.clone()).await;
    graph::verify_postgres_upgrade(&url, &a, &b).await;
    let forged = AccountRepository::postgres(&url, &guest, &accounts[0].id)
        .await
        .unwrap();
    assert!(matches!(
        forged.draft("shared-draft").await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        forged.save_draft(draft(&accounts[0].id)).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(forged.events(0).await, Err(Error::NotFound)));
    assert!(matches!(
        forged.thread_neighbors(&["gmail:42".into()]).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        forged.cache_blob("cache-star").await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        forged.recipient_suggestions("").await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        forged.classification_candidate().await,
        Err(Error::NotFound)
    ));
    assert!(matches!(forged.next_push(&[]).await, Err(Error::NotFound)));
    assert!(matches!(
        forged.recover_models().await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        forged.provider_document("same-key").await,
        Err(Error::NotFound)
    ));
    control
        .revoke_account(&owner, &accounts[0].id)
        .await
        .unwrap();
    if let Backend::Postgres(db) = &a.backend {
        for table in [
            "account_locks",
            "drafts",
            "draft_blobs",
            "blobs",
            "attachment_metadata",
            "submissions",
            "submission_mime",
            "outbox",
            "events",
            "provider_documents",
            "messages",
            "operations",
            "snoozes",
            "rule_inbox_observations",
            "rule_executions",
            "search_dirty",
            "search_documents",
            "search_index_outbox",
            "search_excluded_folders",
            "search_inventory",
            "sync_cursors",
            "provider_refs",
            "push_subscriptions",
            "recipients",
            "search_totals",
            "folder_scans",
            "folder_memberships",
            "notification_dedup",
            "push_jobs",
            "message_identities",
            "model_jobs",
            "thread_verifications",
            "thread_edges",
            "received_cache",
            "recipient_evidence",
        ] {
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE account_id=$1"))
                    .bind(&accounts[0].id)
                    .fetch_one(db)
                    .await
                    .unwrap();
            assert_eq!(count, 0, "removed account retained {table}");
        }
    }
    assert!(matches!(
        a.draft("shared-draft").await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        a.claim_unsubscribe("after-removal").await,
        Err(Error::NotFound)
    ));
    assert_eq!(b.draft("shared-draft").await.unwrap().revision, 1);
    let persisted_cursor = b.event_cursor().await.unwrap();
    let before = b
        .submissions()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.operation_id == "operation")
        .unwrap();
    drop(b);
    let restarted = AccountRepository::postgres(&url, &guest, &accounts[1].id)
        .await
        .unwrap();
    let retry = restarted
        .submit(
            &SubmitRequest {
                operation_id: "operation".into(),
                draft_id: "shared-draft".into(),
                expected_revision: 1,
                due_at: None,
                timezone: "UTC".into(),
            },
            "2099-01-01T00:00:00.000Z",
        )
        .await
        .unwrap();
    assert_eq!(retry, before);
    assert_eq!(restarted.event_cursor().await.unwrap(), persisted_cursor);
}

async fn merged_main_contract(a: &AccountRepository, b: &AccountRepository) {
    let mut original = message(&a.account, "main-merge-classification");
    original.received_at = now();
    original.sender.email = "main-merge@example.invalid".into();
    original.classification = Some(Classification {
        kind: InboxKind::Automated,
        status: CategoryStatus::Ready,
        types: vec![DetectedType::Receipt, DetectedType::Invoice],
        ..Default::default()
    });
    let row = a
        .ingest_messages(vec![original.clone()], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    let mut other = original;
    other.account_id = b.account.clone();
    other.classification = Some(Classification {
        kind: InboxKind::Automated,
        status: CategoryStatus::Ready,
        ..Default::default()
    });
    b.ingest_messages(vec![other], &[], &[], None)
        .await
        .unwrap();
    let filter = ClassificationFilter {
        transactions: true,
        ..Default::default()
    };
    assert!(a
        .inbox_rows(None, None, &filter)
        .await
        .unwrap()
        .iter()
        .any(|m| m.id == row.id));
    assert!(!b
        .inbox_rows(None, None, &filter)
        .await
        .unwrap()
        .iter()
        .any(|m| m.id == row.id));
    let until = (Utc::now() + Duration::days(1)).to_rfc3339();
    a.operation(mutation(
        &row,
        "main-merge-snooze",
        Change::Snooze { until },
    ))
    .await
    .unwrap();
    assert!(a
        .snoozed_rows_filtered(None, None, &filter)
        .await
        .unwrap()
        .iter()
        .any(|m| m.id == row.id));
    let row = a.message(&row.id).await.unwrap();
    let restored = a
        .operation(mutation(
            &row,
            "main-merge-restore",
            Change::Move {
                folder_id: "INBOX".into(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(restored.status, OperationStatus::Applied);
    assert!(!a
        .active_snoozes()
        .await
        .unwrap()
        .iter()
        .any(|v| v["thread_id"] == row.thread_id));
    let row = a.message(&row.id).await.unwrap();
    a.operation(mutation(
        &row,
        "main-merge-sender",
        Change::InboxKind {
            value: Some(InboxKind::Personal),
            sender: true,
        },
    ))
    .await
    .unwrap();
    assert_eq!(
        a.message(&row.id).await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
    assert_eq!(
        b.message(&row.id).await.unwrap().inbox_kind(),
        InboxKind::Automated
    );
    let row = a.message(&row.id).await.unwrap();
    a.operation(mutation(
        &row,
        "main-merge-type",
        Change::DetectedType {
            value: DetectedType::Receipt,
            present: false,
        },
    ))
    .await
    .unwrap();
    let filter = ClassificationFilter {
        email_type: Some(DetectedType::Receipt),
        ..Default::default()
    };
    assert!(!a
        .inbox_rows(None, None, &filter)
        .await
        .unwrap()
        .iter()
        .any(|m| m.id == row.id));
    assert!(a
        .message(&row.id)
        .await
        .unwrap()
        .classification
        .unwrap()
        .types
        .contains(&DetectedType::Invoice));
    a.begin_graph_scan("INBOX").await.unwrap();
    a.commit_graph_page(
        "INBOX",
        &[],
        &[],
        "opaque-merge-cursor",
        false,
        false,
        &[],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        a.provider_document("graph:cursor:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("opaque-merge-cursor")
    );
    assert_eq!(
        b.provider_document("graph:cursor:INBOX").await.unwrap(),
        None
    );
}

async fn graph_delta_contract(a: &AccountRepository, b: &AccountRepository) {
    let mut ids = Vec::new();
    for repository in [a, b] {
        let id = repository
            .message_ids(&["graph:old-copy".into()])
            .await
            .unwrap()
            .remove(0);
        let mut row = message(&repository.account, &id);
        row.provider_id = "graph:old-copy".into();
        repository.ingest_messages(vec![row],&[
            (format!("reference:{id}"),r#"{"graph_id":"new-copy","folder":"INBOX"}"#.into()),
            (format!("thread-headers:{id}"),r#"{"ids":["graph-delta@example.invalid"],"fingerprint":"graph-fingerprint"}"#.into())
        ],&[],None).await.unwrap();
        repository.begin_graph_scan("INBOX").await.unwrap();
        assert_eq!(
            repository
                .graph_identity_candidate(
                    "graph-delta@example.invalid",
                    "graph-fingerprint",
                    "another-copy"
                )
                .await
                .unwrap(),
            Some((id.clone(), "new-copy".into()))
        );
        ids.push(id);
    }
    assert!(a
        .graph_identity_candidate(
            "graph-delta@example.invalid",
            "wrong-fingerprint",
            "another-copy"
        )
        .await
        .unwrap()
        .is_none());
    a.commit_graph_page(
        "INBOX",
        &[],
        &["old-copy".into()],
        "stale-removal",
        false,
        false,
        &[],
        &[],
    )
    .await
    .unwrap();
    assert_eq!(a.message(&ids[0]).await.unwrap().folder_ids, ["INBOX"]);
    a.put_provider_document(
        &format!("reference:{}", ids[0]),
        r#"{"graph_id":"old-copy","folder":"INBOX"}"#,
    )
    .await
    .unwrap();
    a.commit_graph_page(
        "INBOX",
        &[],
        &["old-copy".into()],
        "confirmed-removal",
        false,
        true,
        &[],
        &[],
    )
    .await
    .unwrap();
    assert!(a.message(&ids[0]).await.unwrap().folder_ids.is_empty());
    assert_eq!(b.message(&ids[1]).await.unwrap().folder_ids, ["INBOX"]);
    assert_eq!(
        a.provider_document("graph:cursor:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("confirmed-removal")
    );
}
