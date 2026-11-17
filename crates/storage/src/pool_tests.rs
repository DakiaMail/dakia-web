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
#[path = "search_completion_tests.rs"]
mod search_completion_tests;
#[path = "search_delivery_tests.rs"]
mod search_delivery_tests;
#[path = "search_embedding_tests.rs"]
mod search_embedding_tests;
#[path = "search_filter_tests.rs"]
mod search_filter_tests;
#[path = "search_payload_tests.rs"]
mod search_payload_tests;
#[path = "search_spelling_tests.rs"]
mod search_spelling_tests;
#[path = "search_vector_tests.rs"]
mod search_vector_tests;
#[path = "starred_cache_tests.rs"]
mod starred_cache_tests;
#[path = "store_backend_tests.rs"]
mod store_backend_tests;

#[tokio::test]
async fn bundled_sqlite_keeps_concurrent_page_caches_independent() {
    let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    // MEMORY_MANAGEMENT forces SQLite's global LRU mutex across otherwise
    // private account/search connections. Guard the actual linked library.
    let global_lru: i64 =
        sqlx::query_scalar("SELECT sqlite_compileoption_used('ENABLE_MEMORY_MANAGEMENT')")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(global_lru, 0);
    db.close().await;
}

fn message(a: &AuthorizedAccount, id: &str) -> Message {
    serde_json::from_value(serde_json::json!({"id":id,"account_id":a.id(),"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"Synthetic","preview":"","received_at":"2026-10-29T00:00:00.000Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap()
}

#[tokio::test]
async fn cancelled_writer_begin_does_not_poison_the_account_pool() {
    use sqlx::Connection;
    use std::{
        future::Future,
        task::{Context, Poll},
    };

    struct Ready(tokio::sync::Notify);
    impl futures_util::task::ArcWake for Ready {
        fn wake_by_ref(ready: &Arc<Self>) {
            ready.0.notify_one();
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cancelled.sqlite");
    let db = pool(&path).await.unwrap();
    sqlx::query("CREATE TABLE writes(value INTEGER)")
        .execute(&db)
        .await
        .unwrap();
    let mut connection = db.acquire().await.unwrap();
    let mut begin = Box::pin(connection.begin_with("BEGIN IMMEDIATE"));
    let ready = Arc::new(Ready(tokio::sync::Notify::new()));
    let waker = futures_util::task::waker(ready.clone());
    let mut context = Context::from_waker(&waker);
    assert!(matches!(begin.as_mut().poll(&mut context), Poll::Pending));
    // Let BEGIN execute, then acknowledge it and cancel during handle validation.
    tokio::time::timeout(std::time::Duration::from_secs(5), ready.0.notified())
        .await
        .unwrap();
    assert!(matches!(begin.as_mut().poll(&mut context), Poll::Pending));
    drop(begin);
    connection.ping().await.unwrap();
    assert!(connection.is_in_transaction());
    sqlx::query("INSERT INTO writes VALUES(99)")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);

    let mut next = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("INSERT INTO writes VALUES(1)")
        .execute(&mut *next)
        .await
        .unwrap();
    next.commit().await.unwrap();
    let independent = pool(&path).await.unwrap();
    independent
        .begin_with("BEGIN IMMEDIATE")
        .await
        .unwrap()
        .rollback()
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM writes")
            .fetch_one(&independent)
            .await
            .unwrap(),
        1
    );
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
        search: None,
        supplement: None,
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
        search: None,
        supplement: None,
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

async fn configure_search_fixture(store: &mut Store, url: Option<&str>) {
    store.configure_search_database(url).await.unwrap();
    // Job and retrieval fixtures start after schema initialization. Production
    // configuration intentionally returns while that work runs in the background.
    store
        .search_database
        .as_ref()
        .unwrap()
        .identity()
        .await
        .unwrap();
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

#[path = "search_lexical_tests.rs"]
mod search_lexical_tests;

#[tokio::test]
async fn cancelled_first_open_finishes_initialization() {
    let (dir, store, user, a, _input) = fixture().await;
    let db = store.db(&a).await.unwrap();
    let mut blocker = db.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let scope = reopened.authorize(&user, a.id()).await.unwrap();
    // The caller's deadline must not keep rolling back a slow initial upgrade.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), reopened.db(&scope))
            .await
            .is_err()
    );
    sqlx::query("COMMIT").execute(&mut *blocker).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Ok(pools) = reopened.account_pools.try_lock() {
                if pools.contains_key(scope.id()) {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("initialization must finish after its caller is cancelled");
    assert!(reopened.db(&scope).await.is_ok());
}
