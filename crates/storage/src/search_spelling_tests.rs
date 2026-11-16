use super::*;
async fn contract(url: Option<&str>) {
    let (dir, mut store, user, a, input) = fixture().await;
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let mut m = message(&a, "spelling-source");
    m.subject = "Invoice parcel".into();
    let m = store.upsert_message(&a, m).await.unwrap();
    let mut exact = message(&a, "exact-spelling");
    exact.subject = "parcle".into();
    let exact = store.upsert_message(&a, exact).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_bodies(
            &a,
            &[
                (
                    &m,
                    "courier delivery 東京大阪 доставка",
                    "ready",
                    Some(false),
                ),
                (&exact, "other", "ready", Some(false)),
            ],
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    for (raw, expected) in [
        ("parcle", "parcel"),
        ("xarcel", "parcel"),
        ("delivrey", "delivery"),
        ("東京大坂", "東京大阪"),
        ("достаква", "доставка"),
    ] {
        let units = vec![raw.into()];
        let alternatives = store.search_spelling(&a, &units, &[true]).await.unwrap();
        assert!(
            alternatives[0].iter().any(|v| v == expected),
            "missing {raw} -> {expected}: {alternatives:?}"
        );
        let hits = store
            .full_text_ranked_rows(
                &a,
                &CachedSearchFilter {
                    text_units: Some(&units),
                    text_alternatives: Some(&alternatives),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert!(
            hits.iter().any(|(hit, _)| hit.id == m.id),
            "missing expanded hit {raw}"
        );
        if raw == "parcle" {
            assert_eq!(
                hits[0].0.id, exact.id,
                "literal evidence must outrank a spelling variant"
            );
        }
    }
    let units = vec!["parcle".into()];
    assert_eq!(
        store.search_spelling(&a, &units, &[false]).await.unwrap(),
        vec![units.clone()]
    );
    let alternatives = store.search_spelling(&a, &units, &[true]).await.unwrap();
    let strict = store
        .full_text_ranked_rows(
            &a,
            &CachedSearchFilter {
                text_units: Some(&units),
                text_alternatives: Some(&alternatives),
                from: Some("unrelated"),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    assert!(
        strict.is_empty(),
        "spelling must not weaken structured filters"
    );
    for raw in ["courier delivrey", "financ@example.invalid", "abc", "1234"] {
        let units = vec![raw.into()];
        assert_eq!(
            store.search_spelling(&a, &units, &[true]).await.unwrap(),
            vec![units]
        );
    }
    let account = store.connect(&user, &input).await.unwrap();
    let b = store.authorize(&user, &account.id).await.unwrap();
    let mut foreign = message(&b, "foreign-spelling");
    foreign.subject = "marcle".into();
    store.upsert_message(&b, foreign).await.unwrap();
    store.index_search_headers(&b).await.unwrap();
    store.flush_search_index(&b).await.unwrap();
    let own = store.search_spelling(&a, &units, &[true]).await.unwrap();
    assert!(
        !own[0].iter().any(|v| v == "marcle"),
        "another account must not influence expansion"
    );
    let other = store.search_spelling(&b, &units, &[true]).await.unwrap();
    assert!(other[0].iter().any(|v| v == "marcle"));
    assert!(!other[0].iter().any(|v| v == "parcel"));
    rebuild_old_chunks(&store, &a, url).await;
    drop(store);
    let mut store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = store.authorize(&user, a.id()).await.unwrap();
    let b = store.authorize(&user, b.id()).await.unwrap();
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    assert_eq!(
        store.search_spelling(&a, &units, &[true]).await.unwrap(),
        own,
        "dictionary must survive restart"
    );
    let mut removed = m.clone();
    removed.folder_ids = vec!["Trash".into()];
    store.upsert_message(&a, removed).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let after = store
        .search_spelling(&a, &["xarcel".into()], &[true])
        .await
        .unwrap();
    assert_eq!(
        after,
        vec![vec!["xarcel".to_owned()]],
        "excluded content must leave the spelling dictionary"
    );
    store.remove_account(&a, &input.email).await.unwrap();
    store.remove_account(&b, &input.email).await.unwrap();
    let target = crate::search_backend::SearchDatabase::open(dir.path(), url)
        .await
        .unwrap();
    let retained: i64 = match &target {
        crate::search_backend::SearchDatabase::Sqlite(db) => {
            sqlx::query_scalar("SELECT count(*) FROM spelling_words WHERE account_id IN (?,?)")
                .bind(a.id())
                .bind(b.id())
                .fetch_one(db)
                .await
                .unwrap()
        }
        crate::search_backend::SearchDatabase::Postgres(db) => {
            sqlx::query_scalar("SELECT count(*) FROM spelling_words WHERE account_id IN ($1,$2)")
                .bind(a.id())
                .bind(b.id())
                .fetch_one(db)
                .await
                .unwrap()
        }
    };
    assert_eq!(retained, 0, "account removal must delete spelling data");
}
#[tokio::test]
async fn sqlite_spelling_scope_unicode_strict_filters_restart_and_cleanup() {
    contract(None).await;
}
#[tokio::test]
#[ignore = "requires isolated pgvector PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
async fn postgres_spelling_scope_unicode_strict_filters_restart_and_cleanup() {
    contract(Some(&std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap())).await;
}

async fn rebuild_old_chunks(store: &Store, a: &AuthorizedAccount, url: Option<&str>) {
    let revision = "spelling-upgrade-fixture";
    store.prepare_embeddings(a, revision, true).await.unwrap();
    store.discover_embeddings(a, revision).await.unwrap();
    for _ in 0..16 {
        let Some(turn) = store.claim_embedding(a, revision, true).await.unwrap() else {
            break;
        };
        let mut vector = vec![0.; 384];
        vector[0] = 1.;
        store
            .commit_embedding(
                a,
                &turn,
                &EmbeddedPassage {
                    text: turn.text.clone(),
                    vector: Some(vector),
                    byte_end: turn.text.len(),
                    next_offset: turn.text.len(),
                },
            )
            .await
            .unwrap();
    }
    store.flush_embedding_receipts(a).await.unwrap();
    let target = crate::search_backend::SearchDatabase::open(&store.root, url)
        .await
        .unwrap();
    async fn vectors(
        target: &crate::search_backend::SearchDatabase,
        account: &str,
    ) -> Vec<(i64, String)> {
        match target {
            crate::search_backend::SearchDatabase::Sqlite(db) => sqlx::query_as(
                "SELECT id,hex(embedding) FROM chunks WHERE account_id=? ORDER BY id",
            )
            .bind(account)
            .fetch_all(db)
            .await
            .unwrap(),
            crate::search_backend::SearchDatabase::Postgres(db) => sqlx::query_as(
                "SELECT id,embedding::text FROM chunks WHERE account_id=$1 ORDER BY id",
            )
            .bind(account)
            .fetch_all(db)
            .await
            .unwrap(),
        }
    }
    let before = vectors(&target, a.id()).await;
    assert!(!before.is_empty());
    match &target {
        crate::search_backend::SearchDatabase::Sqlite(db) => {
            for sql in [
                "DELETE FROM spelling_chunk_words WHERE account_id=?",
                "UPDATE lexical_chunks SET revision=2 WHERE account_id=?",
                "UPDATE documents SET lexical_revision=2 WHERE account_id=?",
                "UPDATE lexical_accounts SET cursor='',sweep_done=0 WHERE account_id=?",
            ] {
                sqlx::query(sql).bind(a.id()).execute(db).await.unwrap();
            }
        }
        crate::search_backend::SearchDatabase::Postgres(db) => {
            for sql in [
                "DELETE FROM spelling_chunk_words WHERE account_id=$1",
                "UPDATE lexical_chunks SET revision=2 WHERE account_id=$1",
                "UPDATE documents SET lexical_revision=2 WHERE account_id=$1",
                "UPDATE lexical_accounts SET cursor='',sweep_done=0 WHERE account_id=$1",
            ] {
                sqlx::query(sql).bind(a.id()).execute(db).await.unwrap();
            }
        }
    }
    let source = store.db(a).await.unwrap();
    sqlx::query("UPDATE search_semantic_receipts SET lexical_revision=2")
        .execute(&source)
        .await
        .unwrap();
    assert_eq!(
        store.text_coverage(a).await.unwrap().processed,
        0,
        "old lexical proofs cannot credit the new dictionary"
    );
    assert_eq!(
        store
            .search_spelling(a, &["delivrey".into()], &[true])
            .await
            .unwrap(),
        vec![vec!["delivrey".to_owned()]]
    );
    store.flush_search_index(a).await.unwrap();
    for _ in 0..32 {
        if !store.advance_search_lexical(a, true).await.unwrap() {
            break;
        }
    }
    store.flush_embedding_receipts(a).await.unwrap();
    let coverage = store.text_coverage(a).await.unwrap();
    assert_eq!(coverage.processed, 2);
    assert!(coverage.lexical_discovered);
    assert_eq!(coverage.lexical_pending, 0);
    assert!(store
        .search_spelling(a, &["delivrey".into()], &[true])
        .await
        .unwrap()[0]
        .iter()
        .any(|word| word == "delivery"));
    assert_eq!(
        vectors(&target, a.id()).await,
        before,
        "lexical upgrade must preserve durable embeddings without inference"
    );
}
