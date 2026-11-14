use super::*;
use crate::search_backend::SearchDatabase;
async fn contract(application: Option<&str>, url: Option<&str>) {
    let (dir, _, _, _, input) = fixture().await;
    let root = dir.path().join("lexical");
    let mut store = Store::open_with_database(&root, [31; 32], 1024 * 1024 * 1024, application)
        .await
        .unwrap();
    let sqlite = format!("sqlite:{}", root.join("lexical.sqlite").display());
    let url = Some(url.unwrap_or(&sqlite));
    let user = store
        .user("fixture", &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let m = store
        .upsert_message(&a, message(&a, "long-lexical"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    let body = format!(
        "{} terminal needle {} final phrase 東京 café",
        "garden ".repeat(18000),
        "garden ".repeat(300)
    );
    let attachments = format!(
        "late.pdf attachment only {} final attachment",
        "distinctive ".repeat(110000)
    );
    store
        .put_search_records(
            &a,
            &[SearchContent {
                message: &m,
                text: &body,
                state: "ready",
                participants: "Hidden Friend",
                recipients: "hidden@example.invalid",
                attachments: &attachments,
                attachment_details: "[]",
            }],
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    store
        .prepare_embeddings(&a, "lexical-fixture", true)
        .await
        .unwrap();
    store
        .discover_embeddings(&a, "lexical-fixture")
        .await
        .unwrap();
    for _ in 0..200 {
        let Some(turn) = store
            .claim_embedding(&a, "lexical-fixture", true)
            .await
            .unwrap()
        else {
            break;
        };
        let mut vector = vec![0.; 384];
        vector[0] = 1.;
        let passage = EmbeddedPassage {
            text: turn.text.clone(),
            byte_end: turn.text.len(),
            next_offset: turn.text.len(),
            vector: Some(vector),
        };
        store.commit_embedding(&a, &turn, &passage).await.unwrap();
    }
    store.flush_embedding_receipts(&a).await.unwrap();
    assert_eq!(
        store.text_coverage(&a).await.unwrap().processed,
        0,
        "semantic completion cannot skip unfinished lexical work"
    );
    let target = SearchDatabase::open(&root, url).await.unwrap();
    match &target {
        SearchDatabase::Sqlite(db) => {
            sqlx::query("CREATE TRIGGER lexical_fixture_fault BEFORE UPDATE ON lexical_jobs BEGIN SELECT RAISE(ABORT,'fixture checkpoint fault');END").execute(db).await.unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query("CREATE FUNCTION lexical_fixture_fault() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture checkpoint fault';END $$").execute(db).await.unwrap();
            sqlx::query("CREATE TRIGGER lexical_fixture_fault BEFORE UPDATE ON lexical_jobs FOR EACH ROW EXECUTE FUNCTION lexical_fixture_fault()").execute(db).await.unwrap();
        }
    }
    assert!(store.advance_search_lexical(&a, true).await.is_err());
    assert!(store.text_coverage(&a).await.unwrap().lexical_failed);
    assert!(
        !store.advance_search_lexical(&a, true).await.unwrap(),
        "durable backoff prevents a retry loop"
    );
    match &target {
        SearchDatabase::Sqlite(db) => {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM lexical_chunks WHERE account_id=?"
                )
                .bind(a.id())
                .fetch_one(db)
                .await
                .unwrap(),
                0,
                "chunks and checkpoint roll back together"
            );
            sqlx::query("DROP TRIGGER lexical_fixture_fault")
                .execute(db)
                .await
                .unwrap();
            sqlx::query("UPDATE lexical_accounts SET retry_at=0 WHERE account_id=?")
                .bind(a.id())
                .execute(db)
                .await
                .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM lexical_chunks WHERE account_id=$1"
                )
                .bind(a.id())
                .fetch_one(db)
                .await
                .unwrap(),
                0
            );
            sqlx::query("DROP TRIGGER lexical_fixture_fault ON lexical_jobs")
                .execute(db)
                .await
                .unwrap();
            sqlx::query("DROP FUNCTION lexical_fixture_fault()")
                .execute(db)
                .await
                .unwrap();
            sqlx::query("UPDATE lexical_accounts SET retry_at=0 WHERE account_id=$1")
                .bind(a.id())
                .execute(db)
                .await
                .unwrap();
        }
    }
    assert!(store.advance_search_lexical(&a, true).await.unwrap());
    assert!(!store.text_coverage(&a).await.unwrap().lexical_failed);
    drop(store);
    let mut store = Store::open_with_database(&root, [31; 32], 1024 * 1024 * 1024, application)
        .await
        .unwrap();
    store.configure_search_database(url).await.unwrap();
    let a = store.authorize(&user, a.id()).await.unwrap();
    let mut drained = false;
    for _ in 0..200 {
        if !store.advance_search_lexical(&a, true).await.unwrap() {
            drained = true;
            break;
        }
    }
    assert!(
        drained,
        "bounded lexical continuation must drain after restart"
    );
    assert!(store.flush_embedding_receipts(&a).await.unwrap());
    assert_eq!(store.text_coverage(&a).await.unwrap().processed, 1);
    assert_eq!(store.text_coverage(&a).await.unwrap().lexical_pending, 0);
    for phrase in [
        "terminal needle",
        "garden terminal needle",
        "final phrase",
        "東京 café",
        "attachment only",
        "final attachment",
        "hidden@example.invalid",
    ] {
        let units = vec![phrase.to_owned()];
        let direct = target
            .ranked_rows(
                a.id(),
                &CachedSearchFilter {
                    text: Some(phrase),
                    text_units: Some(&units),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(direct.len(), 1, "target missing {phrase}");
        let rows = store
            .full_text_ranked_rows(
                &a,
                &CachedSearchFilter {
                    text: Some(phrase),
                    text_units: Some(&units),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "missing {phrase}");
    }
    let units = vec!["Friend attachment".to_owned()];
    assert!(
        target
            .ranked_rows(
                a.id(),
                &CachedSearchFilter {
                    text_units: Some(&units),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .is_empty(),
        "phrases cannot cross fields"
    );
    let mut multi_scores = Vec::new();
    for units in [
        vec!["attachment".to_owned(), "Friend".to_owned()],
        vec!["Friend".to_owned(), "attachment".to_owned()],
    ] {
        let rows = target
            .ranked_rows(
                a.id(),
                &CachedSearchFilter {
                    text_units: Some(&units),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "ordinary words may match different fields");
        multi_scores.push(rows[0].2);
    }
    assert_eq!(
        multi_scores[0], multi_scores[1],
        "driving-unit selection must not change scoring"
    );
    for (phrases, expected) in [
        (vec!["terminal needle".into(), "final attachment".into()], 1),
        (vec!["Friend attachment".into()], 0),
        (vec!["final attachment".into()], 1),
        (vec!["final attachment".into(), "absent phrase".into()], 0),
    ] {
        let hits = vec![crate::SemanticMatch {
            message: m.clone(),
            distance: 0.,
            excerpt: String::new(),
            field: 1,
            attachment: None,
        }];
        assert_eq!(
            store
                .constrain_search_phrases(&a, hits, &phrases)
                .await
                .unwrap()
                .len(),
            expected,
            "strict candidate phrase constraints: {phrases:?}"
        );
    }
    let before = match &target {
        SearchDatabase::Sqlite(db) => {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM lexical_chunks WHERE account_id=?")
                .bind(a.id())
                .fetch_one(db)
                .await
                .unwrap()
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM lexical_chunks WHERE account_id=$1")
                .bind(a.id())
                .fetch_one(db)
                .await
                .unwrap()
        }
    };
    assert!(before > 20, "large documents must use bounded chunks");
    let mut flags = m;
    flags.starred = true;
    store.upsert_message(&a, flags).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let after = match &target {
        SearchDatabase::Sqlite(db) => {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM lexical_chunks WHERE account_id=?")
                .bind(a.id())
                .fetch_one(db)
                .await
                .unwrap()
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM lexical_chunks WHERE account_id=$1")
                .bind(a.id())
                .fetch_one(db)
                .await
                .unwrap()
        }
    };
    assert_eq!(before, after, "flag changes retain lexical projection");
    store.remove_account(&a, &input.email).await.unwrap();
}
#[tokio::test]
async fn sqlite_long_lexical_phrases_resume_without_native_limits() {
    contract(None, None).await;
}
#[tokio::test]
#[ignore = "requires disposable pgvector PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
async fn postgres_long_lexical_phrases_resume_without_native_limits() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap();
    contract(None, Some(&url)).await;
}

#[tokio::test]
#[ignore = "requires disposable application/search PostgreSQL URLs"]
async fn all_four_long_lexical_fault_restart_contract() {
    let application = std::env::var("DAKIA_TEST_APPLICATION_DATABASE_URL").unwrap();
    let search = std::env::var("DAKIA_TEST_SEARCH_DATABASE_URL").unwrap();
    contract(Some(&application), None).await;
    contract(Some(&application), Some(&search)).await;
}

#[tokio::test]
async fn small_lexical_upgrade_batches_commit_atomically_and_resume_with_a_bound() {
    let (dir, mut store, _user, a, _) = fixture().await;
    store.configure_search_database(None).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let incoming = (0..9)
        .map(|i| message(&a, &format!("batch-{i:02}")))
        .collect();
    store.ingest_messages(&a, incoming, &[]).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    while store.flush_search_index(&a).await.unwrap() {}
    let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap() else {
        unreachable!()
    };
    sqlx::query("UPDATE lexical_chunks SET revision=3 WHERE account_id=?")
        .bind(a.id())
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE documents SET lexical_revision=3 WHERE account_id=?")
        .bind(a.id())
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE lexical_accounts SET cursor='',sweep_done=0 WHERE account_id=?")
        .bind(a.id())
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER batch_lexical_fault BEFORE UPDATE OF lexical_revision ON documents WHEN NEW.message_id='batch-04' AND NEW.lexical_revision=5 BEGIN SELECT RAISE(ABORT,'synthetic batch fault');END").execute(&db).await.unwrap();
    assert!(store.advance_search_lexical(&a, true).await.is_err());
    let proof:(i64,i64,String)=sqlx::query_as("SELECT (SELECT count(*) FROM lexical_chunks WHERE account_id=?1 AND revision=5),pending,cursor FROM lexical_accounts WHERE account_id=?1").bind(a.id()).fetch_one(&db).await.unwrap();
    assert_eq!(
        proof,
        (0, 0, String::new()),
        "earlier messages, discovery and continuation must roll back with a later failure"
    );
    sqlx::query("DROP TRIGGER batch_lexical_fault")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE lexical_accounts SET retry_at=0 WHERE account_id=?")
        .bind(a.id())
        .execute(&db)
        .await
        .unwrap();
    assert!(store.advance_search_lexical(&a, true).await.unwrap());
    let completed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM documents WHERE account_id=? AND lexical_revision=5",
    )
    .bind(a.id())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
        completed, 8,
        "one turn must leave work beyond its small-message bound"
    );
    assert!(store.advance_search_lexical(&a, true).await.unwrap());
    assert!(!store.advance_search_lexical(&a, true).await.unwrap());
    let coverage = store.text_coverage(&a).await.unwrap();
    assert!(coverage.lexical_discovered);
    assert_eq!(coverage.lexical_pending, 0);
    let completed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM documents WHERE account_id=? AND lexical_revision=5",
    )
    .bind(a.id())
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(completed, 9);
}
