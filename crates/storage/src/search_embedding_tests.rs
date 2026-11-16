use super::*;
use crate::search_backend::SearchDatabase;
fn passage(turn: &EmbeddingTurn, end: usize, next: usize) -> EmbeddedPassage {
    let mut vector = vec![0f32; 384];
    vector[0] = 1.;
    EmbeddedPassage {
        text: turn.text[..end].to_owned(),
        byte_end: end,
        next_offset: next,
        vector: Some(vector),
    }
}
async fn finish(store: &Store, a: &AuthorizedAccount, revision: &str) {
    for _ in 0..200 {
        let Some(turn) = store.claim_embedding(a, revision, true).await.unwrap() else {
            return;
        };
        let chunk = passage(&turn, turn.text.len(), turn.text.len());
        assert!(store.commit_embedding(a, &turn, &chunk).await.unwrap());
    }
    panic!("finite embedding fixture did not drain");
}
async fn rows(db: &SearchDatabase, account: &str) -> Vec<(i64, String, i64, i64)> {
    match db {
        SearchDatabase::Sqlite(db)=>sqlx::query_as("SELECT id,model_revision,content_version,ordinal FROM chunks WHERE account_id=? ORDER BY id").bind(account).fetch_all(db).await.unwrap(),
        SearchDatabase::Postgres(db)=>sqlx::query_as("SELECT id,model_revision,content_version,ordinal FROM chunks WHERE account_id=$1 ORDER BY id").bind(account).fetch_all(db).await.unwrap(),
    }
}
async fn expire(db: &SearchDatabase, account: &str, retry: bool) {
    let field = if retry { "retry_at" } else { "lease_until" };
    match db {
        SearchDatabase::Sqlite(db) => {
            sqlx::query(&format!(
                "UPDATE embedding_jobs SET {field}=0 WHERE account_id=?"
            ))
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query(&format!(
                "UPDATE embedding_jobs SET {field}=0 WHERE account_id=$1"
            ))
            .bind(account)
            .execute(db)
            .await
            .unwrap();
        }
    }
}
async fn fault(db: &SearchDatabase, enabled: bool) {
    match db {
        SearchDatabase::Sqlite(db) => {
            sqlx::query(if enabled {"CREATE TRIGGER embedding_fixture_fault BEFORE UPDATE ON embedding_jobs WHEN OLD.state='running' AND NEW.state='pending' BEGIN SELECT RAISE(ABORT,'fixture chunk checkpoint failure'); END"} else {"DROP TRIGGER embedding_fixture_fault"}).execute(db).await.unwrap();
        }
        SearchDatabase::Postgres(db) => {
            if enabled {
                sqlx::query("CREATE FUNCTION embedding_fixture_fault() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN IF OLD.state='running' AND NEW.state='pending' THEN RAISE EXCEPTION 'fixture chunk checkpoint failure'; END IF; RETURN NEW; END $$").execute(db).await.unwrap();
                sqlx::query("CREATE TRIGGER embedding_fixture_fault BEFORE UPDATE ON embedding_jobs FOR EACH ROW EXECUTE FUNCTION embedding_fixture_fault()").execute(db).await.unwrap();
            } else {
                sqlx::query("DROP TRIGGER embedding_fixture_fault ON embedding_jobs")
                    .execute(db)
                    .await
                    .unwrap();
                sqlx::query("DROP FUNCTION embedding_fixture_fault()")
                    .execute(db)
                    .await
                    .unwrap();
            }
        }
    }
}
async fn fair_history(url: Option<&str>) {
    let (_dir, mut store, _user, a, input) = fixture().await;
    configure_search_fixture(&mut store, url).await;
    let messages = store
        .ingest_messages(
            &a,
            vec![message(&a, "history-one"), message(&a, "history-two")],
            &[],
        )
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    for row in &messages {
        store
            .put_search_body(&a, row, "history body", "ready", Some(false))
            .await
            .unwrap();
    }
    store.flush_search_index(&a).await.unwrap();
    store
        .prepare_embeddings(&a, "fair-history", true)
        .await
        .unwrap();
    store.discover_embeddings(&a, "fair-history").await.unwrap();
    let first = store
        .claim_embedding(&a, "fair-history", true)
        .await
        .unwrap()
        .unwrap();
    assert!(store
        .commit_embedding(&a, &first, &passage(&first, 8, 4))
        .await
        .unwrap());
    let second = store
        .claim_embedding(&a, "fair-history", true)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        first.message_id, second.message_id,
        "history must rotate between partial messages"
    );
    store.remove_account(&a, &input.email).await.unwrap();
}
async fn contract(url: Option<&str>) {
    let (dir, mut store, user, a, input) = fixture().await;
    configure_search_fixture(&mut store, url).await;
    let b = store.connect(&user, &input).await.unwrap();
    let b = store.authorize(&user, &b.id).await.unwrap();
    let m = store
        .upsert_message(&a, message(&a, "semantic"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    let long = format!(
        "{} tailneedle 東京 café",
        "mail delivery русский ".repeat(3000)
    );
    store
        .put_search_records(
            &a,
            &[SearchContent {
                has_attachment: Some(true),
                message: &m,
                text: &long,
                state: "partial",
                participants: "Reply Person reply@example.invalid",
                recipients: "Hidden carbon@example.invalid",
                attachments: "contract.pdf secret attachment",
                attachment_details: "[]",
            }],
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let target = SearchDatabase::open(dir.path(), url).await.unwrap();
    let waiting = store
        .upsert_message(&b, message(&b, "waiting"))
        .await
        .unwrap();
    store.index_search_headers(&b).await.unwrap();
    store.flush_search_index(&b).await.unwrap();
    let rev = "e5-fixture-a";
    store.prepare_embeddings(&a, rev, true).await.unwrap();
    assert!(store.discover_embeddings(&a, rev).await.unwrap());
    let coverage = store.embedding_coverage(&a).await.unwrap();
    assert!(coverage.discovery_complete && coverage.model_available);
    assert_eq!(coverage.pending, 1);
    assert_eq!(coverage.processed, 0);
    store.prepare_embeddings(&b, rev, true).await.unwrap();
    store.discover_embeddings(&b, rev).await.unwrap();
    assert!(store
        .claim_embedding(&b, rev, true)
        .await
        .unwrap()
        .is_none());
    store
        .put_search_body(&b, &waiting, "", "encrypted", Some(false))
        .await
        .unwrap();
    store.flush_search_index(&b).await.unwrap();
    finish(&store, &b, rev).await;
    assert_eq!(store.embedding_coverage(&b).await.unwrap().processed, 1);
    let turn = store
        .claim_embedding(&a, rev, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(turn.field, 0);
    assert!(turn.text.contains("carbon@example.invalid"));
    let chunk = passage(&turn, 8, 4);
    assert!(matches!(
        store.commit_embedding(&b, &turn, &chunk).await,
        Err(Error::NotFound)
    ));
    let mut invalid = chunk.clone();
    invalid.vector.as_mut().unwrap()[0] = f32::NAN;
    assert!(matches!(
        store.commit_embedding(&a, &turn, &invalid).await,
        Err(Error::Invalid)
    ));
    fault(&target, true).await;
    assert!(store.commit_embedding(&a, &turn, &chunk).await.is_err());
    assert!(rows(&target, a.id()).await.is_empty());
    if let SearchDatabase::Sqlite(db) = &target {
        let changes: i64 =
            sqlx::query_scalar("SELECT count(*) FROM embedding_changes WHERE account_id=?")
                .bind(a.id())
                .fetch_one(db)
                .await
                .unwrap();
        assert_eq!(changes, 0);
    }
    fault(&target, false).await;
    assert!(store.commit_embedding(&a, &turn, &chunk).await.unwrap());
    assert!(!store.commit_embedding(&a, &turn, &chunk).await.unwrap());
    let before = rows(&target, a.id()).await;
    assert_eq!(before.len(), 1);
    drop(store);
    let mut store = Store::open(dir.path(), [31; 32]).await.unwrap();
    configure_search_fixture(&mut store, url).await;
    let current = store.authorize(&user, a.id()).await.unwrap();
    assert!(matches!(
        store.claim_embedding(&a, rev, true).await,
        Err(Error::NotFound)
    ));
    let a = current;
    let resumed = store.claim_embedding(&a, rev, true).await.unwrap().unwrap();
    assert_eq!(resumed.byte_offset, 4);
    assert_eq!(resumed.ordinal, 1);
    assert!(store
        .commit_embedding(
            &a,
            &resumed,
            &passage(&resumed, resumed.text.len(), resumed.text.len())
        )
        .await
        .unwrap());
    finish(&store, &a, rev).await;
    let completed = store.embedding_coverage(&a).await.unwrap();
    assert_eq!(completed.processed, 1);
    assert_eq!(completed.pending, 0);
    let original = rows(&target, a.id()).await;
    assert!(original.len() > 3);
    assert!(original.iter().all(|row| row.1 == rev));
    let mut flags = m.clone();
    flags.unread = false;
    store.upsert_message(&a, flags).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert_eq!(rows(&target, a.id()).await, original);
    assert_eq!(store.embedding_coverage(&a).await.unwrap().processed, 1);
    store
        .put_search_body(&a, &m, "replacement body", "ready", Some(false))
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert!(rows(&target, a.id()).await.is_empty());
    assert_eq!(store.embedding_coverage(&a).await.unwrap().processed, 0);
    let mut claims = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let store = store.clone();
        let a = a.clone();
        claims.spawn(async move { store.claim_embedding(&a, rev, true).await.unwrap() });
    }
    let mut claimed = vec![];
    while let Some(result) = claims.join_next().await {
        if let Some(turn) = result.unwrap() {
            claimed.push(turn);
        }
    }
    assert_eq!(claimed.len(), 1);
    let stale = claimed.pop().unwrap();
    expire(&target, a.id(), false).await;
    assert!(!store
        .defer_embedding(&a, &stale, "model_failed")
        .await
        .unwrap());
    let recovered = store.claim_embedding(&a, rev, true).await.unwrap().unwrap();
    assert_ne!(recovered.lease_token, stale.lease_token);
    assert!(!store
        .commit_embedding(
            &a,
            &stale,
            &passage(&stale, stale.text.len(), stale.text.len())
        )
        .await
        .unwrap());
    assert!(store
        .defer_embedding(&a, &recovered, "model_failed")
        .await
        .unwrap());
    assert!(store
        .claim_embedding(&a, rev, true)
        .await
        .unwrap()
        .is_none());
    expire(&target, a.id(), true).await;
    let retried = store.claim_embedding(&a, rev, true).await.unwrap().unwrap();
    assert_eq!(retried.byte_offset, recovered.byte_offset);
    store.prepare_embeddings(&a, rev, false).await.unwrap();
    assert!(!store.embedding_coverage(&a).await.unwrap().model_available);
    let rev2 = "e5-fixture-b";
    store.prepare_embeddings(&a, rev2, true).await.unwrap();
    assert!(!store
        .commit_embedding(
            &a,
            &retried,
            &passage(&retried, retried.text.len(), retried.text.len())
        )
        .await
        .unwrap());
    assert!(store
        .claim_embedding(&a, rev, true)
        .await
        .unwrap()
        .is_none());
    store.discover_embeddings(&a, rev2).await.unwrap();
    let new = store
        .claim_embedding(&a, rev2, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new.ordinal, 0);
    assert_eq!(new.byte_offset, 0);
    assert!(store
        .commit_embedding(&a, &new, &passage(&new, new.text.len(), new.text.len()))
        .await
        .unwrap());
    finish(&store, &a, rev2).await;
    let newer = rows(&target, a.id()).await;
    assert!(newer.iter().all(|row| row.1 == rev2));
    assert!(newer[0].0 > original.last().unwrap().0);
    store
        .put_search_body(&a, &m, "third body", "ready", Some(false))
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let deleted = store
        .claim_embedding(&a, rev2, true)
        .await
        .unwrap()
        .unwrap();
    let mut trash = store.message(&a, &m.id).await.unwrap();
    trash.folder_ids = vec!["All Mail".into(), "Trash".into()];
    store.upsert_message(&a, trash.clone()).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert!(!store
        .commit_embedding(
            &a,
            &deleted,
            &passage(&deleted, deleted.text.len(), deleted.text.len())
        )
        .await
        .unwrap());
    assert!(rows(&target, a.id()).await.is_empty());
    assert_eq!(store.embedding_coverage(&a).await.unwrap().pending, 0);
    trash.folder_ids = vec!["INBOX".into()];
    let restored = store.upsert_message(&a, trash).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &restored, "restored text", "ready", Some(false))
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    finish(&store, &a, rev2).await;
    assert_eq!(store.embedding_coverage(&a).await.unwrap().processed, 1);
    let mut imap = message(&a, "semantic-epoch");
    imap.provider_id = serde_json::json!({"folder":"INBOX","uidvalidity":7,"uid":9}).to_string();
    store
        .put_provider_document(&a, "thread:epoch:INBOX", "7")
        .await
        .unwrap();
    let imap = store.upsert_message(&a, imap).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &imap, "old epoch text", "ready", Some(false))
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let epoch = store
        .claim_embedding(&a, rev2, true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(epoch.message_id, imap.id);
    store
        .put_provider_document(&a, "thread:epoch:INBOX", "8")
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert!(!store
        .commit_embedding(
            &a,
            &epoch,
            &passage(&epoch, epoch.text.len(), epoch.text.len())
        )
        .await
        .unwrap());
    assert_eq!(store.embedding_coverage(&a).await.unwrap().pending, 0);
    store.remove_account(&a, &input.email).await.unwrap();
    assert!(rows(&target, a.id()).await.is_empty());
    assert!(target
        .embedding_coverage(a.id())
        .await
        .unwrap()
        .model_revision
        .is_none());
}
#[tokio::test]
async fn sqlite_embedding_jobs_resume_fence_and_recover() {
    contract(None).await;
    fair_history(None).await;
}
#[tokio::test]
#[ignore = "requires disposable dakia_embedding_test database; DAKIA_TEST_EMBEDDING_POSTGRES_URL"]
async fn postgres_embedding_jobs_resume_fence_and_recover() {
    let url = std::env::var("DAKIA_TEST_EMBEDDING_POSTGRES_URL").unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    assert_eq!(
        options.get_database(),
        Some("dakia_embedding_test"),
        "refuse to reset any other database"
    );
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query("DROP SCHEMA IF EXISTS dakia_search CASCADE")
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    contract(Some(&url)).await;
    fair_history(Some(&url)).await;
}
