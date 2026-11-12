use super::*;
#[path = "classification_pool_tests.rs"]
mod classification_pool_tests;
#[path = "content_reference_tests.rs"]
mod content_reference_tests;
#[path = "full_text_tests.rs"]
mod full_text_tests;
#[path = "idle_notification_tests.rs"]
mod idle_notification_tests;
#[path = "incremental_tests.rs"]
mod incremental_tests;
#[path = "personal_classification_tests.rs"]
mod personal_classification_tests;
#[path = "release_safety_tests.rs"]
mod release_safety_tests;
#[path = "scan_freshness_tests.rs"]
mod scan_freshness_tests;
#[path = "starred_cache_tests.rs"]
mod starred_cache_tests;
fn message(a: &AuthorizedAccount, id: &str) -> Message {
    serde_json::from_value(serde_json::json!({"id":id,"account_id":a.id(),"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"Synthetic","preview":"","received_at":"2026-10-29T00:00:00.000Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap()
}

#[tokio::test]
async fn inbox_keysets_projection_batch_rollback_and_legacy_pruning() {
    let (_dir, store, _user, a, _input) = fixture().await;
    for start in [0, 50] {
        store
            .ingest_messages(
                &a,
                (start..(start + 50).min(70))
                    .map(|i| message(&a, &format!("m{i:03}")))
                    .collect(),
                &[],
            )
            .await
            .unwrap();
    }
    let first = store.inbox_rows(&a, None, None).await.unwrap();
    assert_eq!(first.len(), 51);
    let last = &first[49];
    // Removal/newer arrivals do not shift the keyset continuation.
    let mut removed = first[0].clone();
    removed.folder_ids.clear();
    store.upsert_message(&a, removed).await.unwrap();
    store
        .upsert_message(&a, message(&a, "newer"))
        .await
        .unwrap();
    let next = store
        .inbox_rows(&a, Some((&last.received_at, &last.id)), None)
        .await
        .unwrap();
    assert_eq!(next.len(), 20);
    assert!(!next
        .iter()
        .any(|m| first[..50].iter().any(|old| old.id == m.id)));
    let mut invalid = message(&a, "bad");
    invalid.account_id = "other".into();
    assert!(store
        .ingest_messages(
            &a,
            vec![message(&a, "must-rollback"), invalid],
            &[("test".into(), "must-not-write".into())]
        )
        .await
        .is_err());
    assert!(matches!(
        store.message(&a, "must-rollback").await,
        Err(Error::NotFound)
    ));
    assert_eq!(store.provider_document(&a, "test").await.unwrap(), None);
    store.seed_inbox_memberships(&a).await.unwrap();
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let mut page = SearchPage {
        cache_preview: None,
        index_page: None,
        rows: vec![],
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
            total: 0,
        },
        pending: false,
        counts_exact: false,
    };
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert!(!store.inbox_rows(&a, None, None).await.unwrap().is_empty());
    page.continuation = None;
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert!(store.inbox_rows(&a, None, None).await.unwrap().is_empty());
    assert_eq!(
        store.inbox_coverage(&a).await.unwrap().state,
        CoverageState::Complete
    );
}
#[tokio::test]
async fn complete_reconciliation_does_not_turn_pending_archive_into_provider_deletion() {
    let (_dir, store, _user, a, _input) = fixture().await;
    let m = store
        .upsert_message(&a, message(&a, "pending"))
        .await
        .unwrap();
    store.seed_inbox_memberships(&a).await.unwrap();
    store
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
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let page = SearchPage {
        cache_preview: None,
        index_page: None,
        rows: vec![],
        continuation: None,
        coverage: vec![],
        classification: ClassificationCoverage {
            classified: 0,
            total: 0,
        },
        pending: false,
        counts_exact: false,
    };
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert!(store.inbox_rows(&a, None, None).await.unwrap().is_empty());
    assert!(store
        .message(&a, &m.id)
        .await
        .unwrap()
        .folder_ids
        .contains(&"INBOX".into()));
    let mut claimed = store.claim_mutation(&a).await.unwrap().unwrap();
    claimed.status = OperationStatus::Failed;
    store.finish_mutation(&a, claimed, None).await.unwrap();
    assert_eq!(store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
}

async fn fixture() -> (
    tempfile::TempDir,
    Store,
    String,
    AuthorizedAccount,
    ConnectAccount,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let scope = store.authorize(&user, &account.id).await.unwrap();
    (dir, store, user, scope, input)
}
#[tokio::test]
async fn pooled_accounts_are_isolated_and_credentials_backup_reconnect_do_not_close_shared_handles()
{
    let (dir, store, user, a, input) = fixture().await;
    let account = store.connect(&user, &input).await.unwrap();
    let b = store.authorize(&user, &account.id).await.unwrap();
    store
        .put_provider_document(&a, "test", "account-a")
        .await
        .unwrap();
    let mut tasks = vec![];
    for _ in 0..20 {
        let (store, a) = (store.clone(), a.clone());
        tasks.push(tokio::spawn(async move {
            store.credential(&a).await.unwrap();
            store.provider_document(&a, "test").await.unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().as_deref(), Some("account-a"));
    }
    assert_eq!(store.provider_document(&b, "test").await.unwrap(), None);
    assert_eq!(store.account_pools.lock().await.len(), 2);
    store.reconnect(&a, &input).await.unwrap();
    store.backup(dir.path().join("snapshot")).await.unwrap();
    assert_eq!(
        store
            .provider_document(&a, "test")
            .await
            .unwrap()
            .as_deref(),
        Some("account-a")
    );
    let other = Store::open(dir.path().join("other"), [31; 32])
        .await
        .unwrap();
    assert!(matches!(
        other.provider_document(&a, "test").await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.remove_account(&a, "wrong").await,
        Err(Error::Invalid)
    ));
    assert_eq!(
        store
            .provider_document(&a, "test")
            .await
            .unwrap()
            .as_deref(),
        Some("account-a")
    );
    store.remove_account(&a, &input.email).await.unwrap();
    assert!(matches!(
        store.provider_document(&a, "test").await,
        Err(Error::NotFound)
    ));
    assert!(!dir
        .path()
        .join("accounts")
        .join(format!("{}.sqlite", a.id()))
        .exists());
    assert_eq!(store.account_pools.lock().await.len(), 1);
    assert!(store.provider_document(&b, "test").await.is_ok());
}
#[tokio::test]
async fn failed_open_is_not_cached_and_can_be_retried_after_repair() {
    let (dir, store, user, a, _input) = fixture().await;
    // Disposable migration ledger only. Do not alter production metadata.
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=1")
        .execute(&db)
        .await
        .unwrap();
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a2 = reopened.authorize(&user, a.id()).await.unwrap();
    assert!(matches!(reopened.db(&a2).await, Err(Error::Migration(_))));
    assert!(reopened.account_pools.lock().await.is_empty());
    let migration = sqlx::migrate!("./migrations/account")
        .iter()
        .find(|m| m.version == 1)
        .unwrap()
        .checksum
        .to_vec();
    sqlx::query("UPDATE _sqlx_migrations SET checksum=? WHERE version=1")
        .bind(migration)
        .execute(&db)
        .await
        .unwrap();
    assert!(reopened.db(&a2).await.is_ok());
    assert_eq!(reopened.account_pools.lock().await.len(), 1);
}

#[path = "seed_membership_tests.rs"]
mod seed_membership_tests;
