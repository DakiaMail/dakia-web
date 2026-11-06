use super::*;
#[path = "thread_verification_tests.rs"]
mod verification;

async fn fixture() -> (tempfile::TempDir, Store, AuthorizedAccount) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [23; 32]).await.unwrap();
    let user = store.user("fixture", "thread-storage").await.unwrap();
    let input:ConnectAccount=serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"synthetic-only"})).unwrap();
    let a = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &a.id).await.unwrap();
    (dir, store, a)
}
async fn message(store: &Store, a: &AuthorizedAccount) -> Message {
    let m:Message=serde_json::from_value(serde_json::json!({"id":"fixture","account_id":a.id(),"thread_id":"fixture","provider_id":"gmail:1","folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"Fixture","preview":"","received_at":"2026-10-26T10:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    store.upsert_message(a, m).await.unwrap()
}
#[tokio::test]
async fn index_migrates_existing_headers_and_tracks_replacement_and_cleanup() {
    let (_dir, store, a) = fixture().await;
    message(&store, &a).await;
    let db = store.db(&a).await.unwrap();
    // Recreate a pre-upgrade account containing persisted thread evidence.
    sqlx::raw_sql("DROP TRIGGER thread_headers_insert; DROP TRIGGER thread_headers_update; DROP TRIGGER thread_headers_delete; DROP TRIGGER thread_message_insert; DROP TRIGGER thread_message_update; DROP TRIGGER thread_message_delete; DROP TRIGGER thread_topology_insert; DROP TRIGGER thread_topology_update; DROP TRIGGER thread_topology_delete; DROP TABLE thread_generation; DROP TABLE thread_verifications; DROP TABLE thread_edges; ALTER TABLE folder_scans DROP COLUMN coverage_limited; ALTER TABLE folder_scans DROP COLUMN uidvalidity; DELETE FROM _sqlx_migrations WHERE version=10;").execute(&db).await.unwrap();
    sqlx::query("INSERT INTO provider_documents(key,document) VALUES('thread-headers:fixture',?)")
        .bind(r#"{"ids":["root@example.invalid"],"gmail":42,"fingerprint":"fixture"}"#)
        .execute(&db)
        .await
        .unwrap();
    db.close().await;
    // Migration runs on a real Store/account open, not on every lookup of a
    // retained handle. Reopen the disposable pre-upgrade database explicitly.
    let store = Store::open(_dir.path(), [23; 32]).await.unwrap();
    let a = store.authorize(a.user_id(), a.id()).await.unwrap();
    assert_eq!(
        store
            .thread_neighbors(&a, &["rfc:root@example.invalid".into()])
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .thread_neighbors(&a, &["gmail:42".into()])
            .await
            .unwrap()
            .len(),
        1
    );
    store
        .put_provider_document(
            &a,
            "thread-headers:fixture",
            r#"{"ids":["new@example.invalid"],"gmail":43,"fingerprint":"fixture"}"#,
        )
        .await
        .unwrap();
    assert!(store
        .thread_neighbors(&a, &["gmail:42".into(), "rfc:root@example.invalid".into()])
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .thread_neighbors(&a, &["gmail:43".into()])
            .await
            .unwrap()
            .len(),
        1
    );
    let db = store.db(&a).await.unwrap();
    sqlx::query("DELETE FROM provider_documents WHERE key='thread-headers:fixture'")
        .execute(&db)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM thread_edges")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn partial_coverage_across_pages_cannot_prune_or_certify_a_folder() {
    let (_dir, store, a) = fixture().await;
    let m = message(&store, &a).await;
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let mut page = SearchPage {
        rows: vec![m],
        continuation: None,
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
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    store.reset_scan(&a, "INBOX").await.unwrap();
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    page.rows.clear();
    page.continuation = Some("next".into());
    page.coverage[0].state = CoverageState::Partial;
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    page.continuation = None;
    page.coverage[0].state = CoverageState::Complete;
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert!(store
        .message(&a, "fixture")
        .await
        .unwrap()
        .folder_ids
        .contains(&"INBOX".into()));
    let db = store.db(&a).await.unwrap();
    let completed: Option<i64> =
        sqlx::query_scalar("SELECT completed_at FROM folder_scans WHERE folder='INBOX'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(completed.is_none());
    assert!(store.scan_state(&a, "INBOX").await.unwrap().is_none());
    sqlx::query("UPDATE folder_scans SET last_scanned_at=last_scanned_at-301 WHERE folder='INBOX'")
        .execute(&db)
        .await
        .unwrap();
    let (retry, token) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    assert!(retry > generation && token.is_none());
    // Only a later complete, successful full scan may remove the membership.
    store.record_scan(&a, "INBOX", retry, &page).await.unwrap();
    assert!(store
        .message(&a, "fixture")
        .await
        .unwrap()
        .folder_ids
        .is_empty());
}
