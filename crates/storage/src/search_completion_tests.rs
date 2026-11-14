use super::*;
use crate::search_backend::{completion::CompletionReceipt, SearchDatabase};
const MODEL: &str = "completion-fixture-model";
async fn execute(db: &SearchDatabase, sql: &str) {
    match db {
        SearchDatabase::Sqlite(db) => {
            sqlx::query(sql).execute(db).await.unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query(sql).execute(db).await.unwrap();
        }
    }
}
async fn fault(db: &SearchDatabase, account: &str, source: bool, enabled: bool) {
    Uuid::parse_str(account).unwrap();
    let table = if source {
        "search_semantic_receipts"
    } else {
        "embedding_receipts"
    };
    let action = if source { "INSERT" } else { "DELETE" };
    let row = if source { "NEW" } else { "OLD" };
    match db {
        SearchDatabase::Sqlite(_)=>execute(db,&if enabled {format!("CREATE TRIGGER completion_fixture_fault BEFORE {action} ON {table} BEGIN SELECT RAISE(ABORT,'fixture completion persistence failed');END")}else{"DROP TRIGGER completion_fixture_fault".into()}).await,
        SearchDatabase::Postgres(_)=>{
            if enabled {
                execute(db,&format!("CREATE FUNCTION completion_fixture_fault() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN IF {row}.account_id='{account}' THEN RAISE EXCEPTION 'fixture completion persistence failed';END IF;RETURN {row};END $$")).await;
                execute(db,&format!("CREATE TRIGGER completion_fixture_fault BEFORE {action} ON {table} FOR EACH ROW EXECUTE FUNCTION completion_fixture_fault()")).await;
            }else {execute(db,&format!("DROP TRIGGER completion_fixture_fault ON {table}")).await;execute(db,"DROP FUNCTION completion_fixture_fault()").await;}
        }
    }
}
async fn drain(store: &Store, a: &AuthorizedAccount, revision: &str) {
    store.discover_embeddings(a, revision).await.unwrap();
    for _ in 0..100 {
        let Some(turn) = store.claim_embedding(a, revision, true).await.unwrap() else {
            return;
        };
        let mut vector = vec![0.; 384];
        vector[0] = 1.;
        let passage = EmbeddedPassage {
            text: turn.text.clone(),
            byte_end: turn.text.len(),
            next_offset: turn.text.len(),
            vector: Some(vector),
        };
        assert!(store.commit_embedding(a, &turn, &passage).await.unwrap());
    }
    panic!("completion fixture did not drain");
}
async fn one_message(store: &Store, a: &AuthorizedAccount, revision: &str) -> String {
    let mut id = None;
    for _ in 0..10 {
        let turn = store
            .claim_embedding(a, revision, false)
            .await
            .unwrap()
            .unwrap();
        assert!(id.as_ref().is_none_or(|id| id == &turn.message_id));
        id = Some(turn.message_id.clone());
        let mut vector = vec![0.; 384];
        vector[0] = 1.;
        let passage = EmbeddedPassage {
            text: turn.text.clone(),
            byte_end: turn.text.len(),
            next_offset: turn.text.len(),
            vector: Some(vector),
        };
        store.commit_embedding(a, &turn, &passage).await.unwrap();
        if store.embedding_coverage(a).await.unwrap().processed == 1 {
            return id.unwrap();
        }
    }
    panic!("fixture message did not finish");
}
async fn receipts(db: &SearchDatabase, account: &str) -> Vec<CompletionReceipt> {
    db.completion_receipts(account).await.unwrap()
}
async fn counters(
    store: &Store,
    a: &AuthorizedAccount,
    processed: i64,
    eligible: i64,
    backlog: i64,
) {
    let coverage = store.indexing_inventory(a).await.unwrap();
    assert_eq!(
        (coverage.processed, coverage.eligible, coverage.backlog),
        (processed, eligible, backlog)
    );
    assert!(coverage.semantic_required);
}
async fn contract(application: Option<&str>, search: Option<&str>) {
    let (dir, _, _, _, input) = fixture().await;
    let root = dir.path().join("completion");
    let mut store = Store::open_with_database(&root, [47; 32], 1024 * 1024 * 1024, application)
        .await
        .unwrap();
    store.require_search_embeddings(MODEL).unwrap();
    let sqlite = format!("sqlite:{}", root.join("search.sqlite").display());
    let search = search.unwrap_or(&sqlite);
    store.configure_search_database(Some(search)).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let target = SearchDatabase::open(&root, Some(search)).await.unwrap();
    let user = store
        .user("fixture", &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let guest = store
        .user("fixture", &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    assert!(store.authorize(&guest, &account.id).await.is_err());
    let mut messages = store
        .ingest_messages(
            &a,
            vec![
                message(&a, "completion-a"),
                message(&a, "completion-b"),
                message(&a, "completion-gap"),
            ],
            &[],
        )
        .await
        .unwrap();
    counters(&store, &a, 0, 3, 3).await;
    assert!(
        !store
            .indexing_inventory(&a)
            .await
            .unwrap()
            .semantic_available
    );
    assert!(store
        .prepare_embeddings(&a, "unexpected-model", true)
        .await
        .is_err());
    store.index_search_headers(&a).await.unwrap();
    for m in &messages {
        store
            .put_search_body(
                &a,
                m,
                if m.id.contains("gap") {
                    ""
                } else {
                    "complete retained body"
                },
                if m.id.contains("gap") {
                    "encrypted"
                } else {
                    "ready"
                },
            )
            .await
            .unwrap();
    }
    store.flush_search_index(&a).await.unwrap();
    counters(&store, &a, 0, 3, 3).await;
    store.prepare_embeddings(&a, MODEL, true).await.unwrap();
    store.discover_embeddings(&a, MODEL).await.unwrap();
    let first = one_message(&store, &a, MODEL).await;
    let pending = receipts(&target, a.id()).await;
    assert_eq!(pending.len(), 1);
    // Search storage finishing a stage does not count before source persistence.
    counters(&store, &a, 0, 3, 3).await;
    let repository = store.account_repository(&a).await.unwrap();
    let source = if let Some(url) = application {
        let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        SearchDatabase::Postgres(
            sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options.options([("search_path", "dakia_mail,public")]))
                .await
                .unwrap(),
        )
    } else {
        SearchDatabase::Sqlite(store.db(&a).await.unwrap())
    };
    let identity = target.identity().await.unwrap();
    let mut future = pending[0].clone();
    future.lexical_revision = crate::lexical_text::REVISION + 1;
    assert!(
        repository
            .accept_embedding_receipts(&identity, Some(MODEL), &[future])
            .await
            .unwrap()
            .is_empty(),
        "older runtimes must not acknowledge future lexical receipts"
    );
    // A concurrently newer lexical runtime owns the higher source requirement.
    // Preparing this older runtime must not downgrade its progress profile.
    match &source {
        SearchDatabase::Sqlite(db) => {
            sqlx::query("UPDATE search_semantic_profile SET lexical_revision=? WHERE id=1")
                .bind(crate::lexical_text::REVISION + 1)
                .execute(db)
                .await
                .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query(
                "UPDATE search_semantic_profile SET lexical_revision=$2 WHERE account_id=$1",
            )
            .bind(a.id())
            .bind(crate::lexical_text::REVISION + 1)
            .execute(db)
            .await
            .unwrap();
        }
    }
    repository.semantic_profile(MODEL, true).await.unwrap();
    let requirement: i64 = match &source {
        SearchDatabase::Sqlite(db) => {
            sqlx::query_scalar("SELECT lexical_revision FROM search_semantic_profile WHERE id=1")
                .fetch_one(db)
                .await
                .unwrap()
        }
        SearchDatabase::Postgres(db) => sqlx::query_scalar(
            "SELECT lexical_revision FROM search_semantic_profile WHERE account_id=$1",
        )
        .bind(a.id())
        .fetch_one(db)
        .await
        .unwrap(),
    };
    assert_eq!(requirement, crate::lexical_text::REVISION + 1);
    match &source {
        SearchDatabase::Sqlite(db) => {
            sqlx::query("UPDATE search_semantic_profile SET lexical_revision=? WHERE id=1")
                .bind(crate::lexical_text::REVISION)
                .execute(db)
                .await
                .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query(
                "UPDATE search_semantic_profile SET lexical_revision=$2 WHERE account_id=$1",
            )
            .bind(a.id())
            .bind(crate::lexical_text::REVISION)
            .execute(db)
            .await
            .unwrap();
        }
    }
    fault(&source, a.id(), true, true).await;
    assert!(store.flush_embedding_receipts(&a).await.is_err());
    counters(&store, &a, 0, 3, 3).await;
    assert_eq!(receipts(&target, a.id()).await.len(), 1);
    fault(&source, a.id(), true, false).await;
    assert_eq!(
        repository
            .accept_embedding_receipts(&identity, Some(MODEL), &pending)
            .await
            .unwrap()
            .len(),
        1
    );
    counters(&store, &a, 1, 3, 2).await;
    // Simulate a lost target ACK. Replaying the same receipt is idempotent.
    fault(&target, a.id(), false, true).await;
    assert!(store.flush_embedding_receipts(&a).await.is_err());
    counters(&store, &a, 1, 3, 2).await;
    assert_eq!(receipts(&target, a.id()).await.len(), 1);
    fault(&target, a.id(), false, false).await;
    assert!(store.flush_embedding_receipts(&a).await.unwrap());
    counters(&store, &a, 1, 3, 2).await;
    assert!(receipts(&target, a.id()).await.is_empty());
    // Source and target aggregate completion now describe disjoint message
    // states. Current publication identity, rather than min(counts), governs.
    let first_message = messages.iter_mut().find(|m| m.id == first).unwrap();
    first_message.starred = true;
    *first_message = store
        .upsert_message(&a, first_message.clone())
        .await
        .unwrap();
    counters(&store, &a, 0, 3, 3).await;
    assert!(repository
        .accept_embedding_receipts(&identity, Some(MODEL), &pending)
        .await
        .unwrap()
        .is_empty());
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let newer = receipts(&target, a.id()).await;
    assert_eq!(newer.len(), 1);
    assert!(newer[0].version > pending[0].version);
    // A delayed old ACK must not delete the newer completion publication.
    target
        .acknowledge_completion(a.id(), &pending)
        .await
        .unwrap();
    assert_eq!(receipts(&target, a.id()).await.len(), 1);
    store.flush_embedding_receipts(&a).await.unwrap();
    counters(&store, &a, 1, 3, 2).await;
    drain(&store, &a, MODEL).await;
    store.flush_embedding_receipts(&a).await.unwrap();
    counters(&store, &a, 3, 3, 0).await;
    assert_eq!(store.indexing_inventory(&a).await.unwrap().gaps, 1);
    // Upgrade invalidates old lexical proofs immediately using versioned
    // counters, then republishes a bounded metadata page without re-inference.
    match &source {
        SearchDatabase::Sqlite(db) => {
            sqlx::query("UPDATE search_semantic_receipts SET lexical_revision=0")
                .execute(db)
                .await
                .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query(
                "UPDATE search_semantic_receipts SET lexical_revision=0 WHERE account_id=$1",
            )
            .bind(a.id())
            .execute(db)
            .await
            .unwrap();
        }
    }
    counters(&store, &a, 0, 3, 3).await;
    store.flush_search_index(&a).await.unwrap();
    assert!(store
        .claim_embedding(&a, MODEL, true)
        .await
        .unwrap()
        .is_none());
    store.flush_embedding_receipts(&a).await.unwrap();
    counters(&store, &a, 3, 3, 0).await;
    // A different lexical implementation cannot reuse this process's proofs.
    match &target {
        SearchDatabase::Sqlite(db) => {
            sqlx::query("UPDATE lexical_accounts SET revision=999 WHERE account_id=?")
                .bind(a.id())
                .execute(db)
                .await
                .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query("UPDATE lexical_accounts SET revision=999 WHERE account_id=$1")
                .bind(a.id())
                .execute(db)
                .await
                .unwrap();
        }
    }
    let mismatch = store.text_coverage(&a).await.unwrap();
    assert_eq!((mismatch.processed, mismatch.backlog), (0, 3));
    assert!(mismatch.lexical_failed);
    match &target {
        SearchDatabase::Sqlite(db) => {
            sqlx::query("UPDATE lexical_accounts SET revision=? WHERE account_id=?")
                .bind(crate::lexical_text::REVISION)
                .bind(a.id())
                .execute(db)
                .await
                .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query("UPDATE lexical_accounts SET revision=$2 WHERE account_id=$1")
                .bind(a.id())
                .bind(crate::lexical_text::REVISION)
                .execute(db)
                .await
                .unwrap();
        }
    }
    // A different process selecting another model cannot make this runtime's
    // source receipts claim that its own semantic retrieval is caught up.
    target
        .prepare_embeddings(a.id(), "foreign-model", true)
        .await
        .unwrap();
    let mismatch = store.text_coverage(&a).await.unwrap();
    assert_eq!((mismatch.processed, mismatch.backlog), (0, 3));
    assert!(mismatch.semantic_failed);
    target
        .prepare_embeddings(a.id(), MODEL, true)
        .await
        .unwrap();
    // Component failures must not discard healthy lexical candidates.
    execute(
        &target,
        "ALTER TABLE embedding_progress RENAME TO completion_fixture_progress",
    )
    .await;
    let degraded = store.text_coverage(&a).await.unwrap();
    assert!(degraded.semantic_failed);
    assert_eq!(
        store
            .full_text_ranked_rows(
                &a,
                &CachedSearchFilter {
                    text: Some("complete"),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .len(),
        2
    );
    execute(
        &target,
        "ALTER TABLE completion_fixture_progress RENAME TO embedding_progress",
    )
    .await;
    // Deliver a newer metadata publication and ACK it before an older worker
    // returns. An old lexical ACK must never lower the source delivery version.
    first_message.unread = !first_message.unread;
    *first_message = store
        .upsert_message(&a, first_message.clone())
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    let older = repository.search_deliveries().await.unwrap();
    target.deliver_batch(a.id(), &older).await.unwrap();
    first_message.unread = !first_message.unread;
    *first_message = store
        .upsert_message(&a, first_message.clone())
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    let latest = repository.search_deliveries().await.unwrap();
    target.deliver_batch(a.id(), &latest).await.unwrap();
    repository
        .acknowledge_search_deliveries(&latest)
        .await
        .unwrap();
    repository
        .acknowledge_search_deliveries(&older)
        .await
        .unwrap();
    counters(&store, &a, 2, 3, 1).await;
    store.flush_embedding_receipts(&a).await.unwrap();
    counters(&store, &a, 3, 3, 0).await;
    // Simulate an installation upgraded from before delivery versions existed.
    // Bounded metadata republication preserves target text and existing vectors.
    match &source {
        SearchDatabase::Sqlite(db) => {
            sqlx::query("DELETE FROM search_semantic_receipts")
                .execute(db)
                .await
                .unwrap();
            sqlx::query("UPDATE search_documents SET delivered_version=0")
                .execute(db)
                .await
                .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query("DELETE FROM search_semantic_receipts WHERE account_id=$1")
                .bind(a.id())
                .execute(db)
                .await
                .unwrap();
            sqlx::query("UPDATE search_documents SET delivered_version=0 WHERE account_id=$1")
                .bind(a.id())
                .execute(db)
                .await
                .unwrap();
        }
    }
    counters(&store, &a, 0, 3, 3).await;
    store.flush_search_index(&a).await.unwrap();
    assert!(store
        .claim_embedding(&a, MODEL, true)
        .await
        .unwrap()
        .is_none());
    store.flush_embedding_receipts(&a).await.unwrap();
    counters(&store, &a, 3, 3, 0).await;
    // A terminal content gap finishes processing and does not run forever.
    store.prepare_embeddings(&a, MODEL, false).await.unwrap();
    counters(&store, &a, 3, 3, 0).await;
    assert!(!store.text_coverage(&a).await.unwrap().semantic_available);
    store.prepare_embeddings(&a, MODEL, true).await.unwrap();
    let next = "completion-new-model";
    store.require_search_embeddings(next).unwrap();
    store.prepare_embeddings(&a, next, true).await.unwrap();
    counters(&store, &a, 0, 3, 3).await;
    drain(&store, &a, next).await;
    store.flush_embedding_receipts(&a).await.unwrap();
    counters(&store, &a, 3, 3, 0).await;
    // Trash via an alternate membership invalidates receipts immediately. Dirty
    // and outbox cleanup for this one excluded message count as one backlog item.
    first_message.folder_ids = vec!["INBOX".into(), "All Mail".into(), "Trash".into()];
    *first_message = store
        .upsert_message(&a, first_message.clone())
        .await
        .unwrap();
    counters(&store, &a, 2, 2, 1).await;
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    counters(&store, &a, 2, 2, 0).await;
    first_message.folder_ids = vec!["INBOX".into()];
    *first_message = store
        .upsert_message(&a, first_message.clone())
        .await
        .unwrap();
    counters(&store, &a, 2, 3, 1).await;
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, first_message, "restored body", "ready")
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    drain(&store, &a, next).await;
    // Restart between target commit and receipt delivery retains the backlog.
    let mut reopened = Store::open_with_database(&root, [47; 32], 1024 * 1024 * 1024, application)
        .await
        .unwrap();
    reopened.require_search_embeddings(next).unwrap();
    reopened
        .configure_search_database(Some(search))
        .await
        .unwrap();
    assert!(reopened.indexing_inventory(&a).await.is_err());
    let fresh = reopened.authorize(&user, &account.id).await.unwrap();
    counters(&reopened, &fresh, 2, 3, 1).await;
    reopened.flush_embedding_receipts(&fresh).await.unwrap();
    counters(&reopened, &fresh, 3, 3, 0).await;
    // Epoch changes while a message is already dirty must update cleanup
    // eligibility without adding duplicate backlog items.
    reopened
        .put_provider_document(&fresh, "thread:epoch:INBOX", "7")
        .await
        .unwrap();
    let mut epoch = message(&fresh, "completion-epoch");
    epoch.provider_id = serde_json::json!({"folder":"INBOX","uidvalidity":7,"uid":9}).to_string();
    let epoch = reopened.upsert_message(&fresh, epoch).await.unwrap();
    reopened.index_search_headers(&fresh).await.unwrap();
    reopened
        .put_search_body(&fresh, &epoch, "epoch body", "ready")
        .await
        .unwrap();
    reopened.flush_search_index(&fresh).await.unwrap();
    drain(&reopened, &fresh, next).await;
    reopened.flush_embedding_receipts(&fresh).await.unwrap();
    counters(&reopened, &fresh, 4, 4, 0).await;
    reopened
        .put_provider_document(&fresh, "thread:epoch:INBOX", "8")
        .await
        .unwrap();
    counters(&reopened, &fresh, 3, 3, 1).await;
    reopened
        .put_provider_document(&fresh, "thread:epoch:INBOX", "9")
        .await
        .unwrap();
    counters(&reopened, &fresh, 3, 3, 1).await;
    reopened
        .set_search_excluded_folders(&fresh, &["INBOX".into()])
        .await
        .unwrap();
    counters(&reopened, &fresh, 0, 0, 4).await;
    reopened.index_search_headers(&fresh).await.unwrap();
    reopened.flush_search_index(&fresh).await.unwrap();
    counters(&reopened, &fresh, 0, 0, 0).await;
    reopened
        .set_search_excluded_folders(&fresh, &[])
        .await
        .unwrap();
    counters(&reopened, &fresh, 0, 3, 4).await;
    reopened.index_search_headers(&fresh).await.unwrap();
    for message in &messages {
        let current = reopened.message(&fresh, &message.id).await.unwrap();
        reopened
            .put_search_body(&fresh, &current, "recovered body", "ready")
            .await
            .unwrap();
    }
    reopened.flush_search_index(&fresh).await.unwrap();
    drain(&reopened, &fresh, next).await;
    reopened.flush_embedding_receipts(&fresh).await.unwrap();
    counters(&reopened, &fresh, 3, 3, 0).await;
    reopened.remove_account(&fresh, &input.email).await.unwrap();
    assert!(store.indexing_inventory(&a).await.is_err());
    assert!(target.completion_receipts(a.id()).await.unwrap().is_empty());
}
#[tokio::test]
async fn sqlite_completion_receipts_are_exact_resumable_and_versioned() {
    contract(None, None).await;
}
#[tokio::test]
#[ignore = "requires disposable application and search PostgreSQL URLs"]
async fn all_four_completion_receipt_contract() {
    let application = std::env::var("DAKIA_TEST_APPLICATION_DATABASE_URL").unwrap();
    let search = std::env::var("DAKIA_TEST_SEARCH_DATABASE_URL").unwrap();
    contract(None, Some(&search)).await;
    contract(Some(&application), None).await;
    contract(Some(&application), Some(&search)).await;
}
