use dakia_domain::*;
use dakia_storage::Store;
async fn fixture() -> (
    tempfile::TempDir,
    Store,
    dakia_storage::AuthorizedAccount,
    Message,
) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(d.path(), [9; 32]).await.unwrap();
    let u = s.user("issuer", "owner").await.unwrap();
    let input=serde_json::from_value(serde_json::json!({"email":"test@example.invalid","display_name":"test","provider":"imap","imap_host":"imap.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.invalid","smtp_port":465,"smtp_tls":"implicit","username":"test","password":"fixture-only"})).unwrap();
    let account = s.connect(&u, &input).await.unwrap();
    let a = s.authorize(&u, &account.id).await.unwrap();
    let m:Message=serde_json::from_value(serde_json::json!({"id":"message","account_id":a.id(),"thread_id":"thread","provider_id":"stable","folder_ids":["INBOX","Archive"],"sender":{"email":"test@example.invalid","name":null},"to":[],"subject":"synthetic","preview":"","received_at":"2026-10-28T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    let m = s.upsert_message(&a, m).await.unwrap();
    (d, s, a, m)
}
#[tokio::test]
async fn identity_alias_migration_preserves_existing_mappings() {
    let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql("CREATE TABLE message_identities (provider_id TEXT PRIMARY KEY, message_id TEXT NOT NULL UNIQUE); INSERT INTO message_identities VALUES ('original', 'local-message');")
        .execute(&db).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../migrations/account/0009_message_identity_aliases.sql"
    ))
    .execute(&db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO message_identities VALUES ('moved', 'local-message')")
        .execute(&db)
        .await
        .unwrap();
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT message_id FROM message_identities ORDER BY provider_id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(ids, vec!["local-message", "local-message"]);
    assert!(
        sqlx::query("INSERT INTO message_identities VALUES ('original', 'different')")
            .execute(&db)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn moved_imap_references_keep_one_local_identity_across_reopen() {
    let (d, s, a, m) = fixture().await;
    s.link_identity(&a, "inbox-reference", &m.id).await.unwrap();
    s.link_identity(&a, "archive-reference", &m.id)
        .await
        .unwrap();
    s.link_identity(&a, "restored-inbox-reference", &m.id)
        .await
        .unwrap();
    let reopened = Store::open(d.path(), [9; 32]).await.unwrap();
    let a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    for reference in [
        "inbox-reference",
        "archive-reference",
        "restored-inbox-reference",
    ] {
        assert_eq!(reopened.message_id(&a, reference).await.unwrap(), m.id);
    }
    assert_ne!(
        reopened
            .message_id(&a, "unrelated-reference")
            .await
            .unwrap(),
        m.id
    );
}
#[tokio::test]
async fn mutation_claim_dedup_and_crash_never_replays_move() {
    let (_d, s, a, m) = fixture().await;
    let op = Operation {
        id: "move-once".into(),
        account_id: a.id().into(),
        target_id: m.id.clone(),
        expected_revision: m.revision,
        change: Change::Move {
            folder_id: "Archive".into(),
        },
        status: OperationStatus::Pending,
        revision: 0,
    };
    let stored = s.operation(&a, op.clone()).await.unwrap();
    assert_eq!(s.operation(&a, op).await.unwrap(), stored);
    assert_eq!(s.claim_mutation(&a).await.unwrap().unwrap().id, stored.id);
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    s.recover_mutations(&a).await.unwrap();
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    let mut next = stored.clone();
    next.expected_revision = s.message(&a, &m.id).await.unwrap().revision;
    next.id = "second".into();
    s.operation(&a, next).await.unwrap();
    let mut claimed = s.claim_mutation(&a).await.unwrap().unwrap();
    claimed.status = OperationStatus::Applied;
    s.finish_mutation(&a, claimed, Some(vec!["Archive".into()]))
        .await
        .unwrap();
    assert_eq!(
        s.message(&a, &m.id).await.unwrap().folder_ids,
        vec!["Archive"]
    );
}
#[tokio::test]
async fn partial_scan_restart_and_failed_scan_do_not_prune_other_memberships() {
    let (d, s, a, m) = fixture().await;
    let (generation, _) = s.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let page = SearchPage {
        rows: vec![m.clone()],
        continuation: Some("next".into()),
        coverage: vec![AccountCoverage {
            account_id: a.id().into(),
            folder_ids: vec!["INBOX".into()],
            inbox: None,
            state: CoverageState::Complete,
            error: None,
        }],
        classification: ClassificationCoverage {
            classified: 0,
            total: 1,
        },
        pending: false,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
    };
    s.record_scan(&a, "INBOX", generation, &page).await.unwrap();
    let reopened = Store::open(d.path(), [9; 32]).await.unwrap();
    let a2 = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    assert_eq!(
        reopened.scan_state(&a2, "INBOX").await.unwrap().unwrap().1,
        Some("next".into())
    );
    // A UIDVALIDITY reset abandons the generation, retaining metadata until the next successful full scan.
    reopened.reset_scan(&a2, "INBOX").await.unwrap();
    assert_eq!(
        reopened.message(&a2, &m.id).await.unwrap().folder_ids.len(),
        2
    );
    let (generation, _) = reopened.scan_state(&a2, "INBOX").await.unwrap().unwrap();
    let mut empty = page;
    empty.rows.clear();
    empty.continuation = None;
    reopened
        .record_scan(&a2, "INBOX", generation, &empty)
        .await
        .unwrap();
    assert_eq!(
        reopened.message(&a2, &m.id).await.unwrap().folder_ids,
        vec!["Archive"]
    );
    assert!(reopened.scan_state(&a2, "INBOX").await.unwrap().is_none());
}
#[tokio::test]
async fn identical_metadata_does_not_emit_spurious_revision_or_erase_classification() {
    let (_d, s, a, mut m) = fixture().await;
    m.category = Some("Personal".into());
    m.category_status = CategoryStatus::Ready;
    let stored = s.upsert_message(&a, m.clone()).await.unwrap();
    m.category = None;
    m.category_status = CategoryStatus::Pending;
    assert_eq!(s.upsert_message(&a, m).await.unwrap(), stored);
}

#[tokio::test]
async fn queued_archive_survives_reload_without_changing_provider_state_and_failure_unhides() {
    let (d, s, a, m) = fixture().await;
    let op = s
        .operation(
            &a,
            Operation {
                id: "archive".into(),
                account_id: a.id().into(),
                target_id: m.id.clone(),
                expected_revision: m.revision,
                change: Change::Archive,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        s.operation_status(&a, &op.id).await.unwrap().status,
        OperationStatus::Pending
    );
    let reopened = Store::open(d.path(), [9; 32]).await.unwrap();
    let a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    let observed = reopened.message(&a, &m.id).await.unwrap();
    assert!(observed.folder_ids.contains(&"INBOX".into()));
    assert_eq!(
        reopened
            .visible_message(&a, observed.clone())
            .await
            .unwrap()
            .folder_ids,
        vec!["Archive"]
    );
    let mut claimed = reopened.claim_mutation(&a).await.unwrap().unwrap();
    claimed.status = OperationStatus::Failed;
    reopened.finish_mutation(&a, claimed, None).await.unwrap();
    assert_eq!(
        reopened.operation_status(&a, &op.id).await.unwrap().status,
        OperationStatus::Failed
    );
    assert!(reopened
        .visible_message(&a, observed)
        .await
        .unwrap()
        .folder_ids
        .contains(&"INBOX".into()));
}
