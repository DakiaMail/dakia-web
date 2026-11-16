use super::*;
use crate::{search_backend::SearchDatabase, search_vectors::VectorAccelerator};

async fn catch_up(accelerator: &VectorAccelerator, db: &SqlitePool) {
    for _ in 0..100 {
        if !accelerator.advance(db).await.unwrap() {
            return;
        }
    }
    panic!("bounded vector fixture did not catch up");
}
fn axis(n: usize) -> Vec<f32> {
    let mut v = vec![0.; 384];
    v[n] = 1.;
    v
}
#[tokio::test]
async fn semantic_candidates_expand_past_a_long_documents_duplicate_chunks() {
    adaptive_cohort(false, false).await;
}
#[tokio::test]
async fn identical_vector_chunks_do_not_hide_other_messages() {
    adaptive_cohort(true, false).await;
}
#[tokio::test]
async fn exact_filtered_cohort_deduplicates_before_limiting_chunks_without_an_accelerator() {
    adaptive_cohort(false, true).await;
}
async fn adaptive_cohort(identical: bool, exact: bool) {
    let (dir, mut store, _user, a, _) = fixture().await;
    configure_search_fixture(&mut store, None).await;
    let mut incoming = vec![message(&a, "adaptive-long")];
    incoming.extend((0..20).map(|i| message(&a, &format!("adaptive-{i:02}"))));
    let messages = store.ingest_messages(&a, incoming, &[]).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    for m in &messages {
        store
            .put_search_body(&a, m, "searchable passage", "ready", Some(false))
            .await
            .unwrap();
    }
    while store.flush_search_index(&a).await.unwrap() {}
    let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap() else {
        unreachable!()
    };
    let bytes = |v: &[f32]| v.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
    let mut tx = db.begin().await.unwrap();
    let chunks = if exact { 3000 } else { 700 };
    for ordinal in 0..chunks {
        let mut vector = axis(0);
        if !identical {
            vector[1] = ordinal as f32 / 10000.;
        }
        sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT account_id,message_id,source,'adaptive-fixture',?,'long passage',?,content_version,1 FROM documents WHERE account_id=? AND message_id='adaptive-long'")
            .bind(ordinal).bind(bytes(&vector)).bind(a.id()).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    for i in 0..20 {
        let mut vector = axis(0);
        vector[1] = 0.2 + i as f32 / 100.;
        sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT account_id,message_id,source,'adaptive-fixture',0,'later match',?,content_version,1 FROM documents WHERE account_id=? AND message_id=?")
            .bind(bytes(&vector)).bind(a.id()).bind(format!("adaptive-{i:02}")).execute(&db).await.unwrap();
    }
    let inventory: (i64,i64) = sqlx::query_as("SELECT count(*),count(DISTINCT message_id) FROM chunks WHERE account_id=? AND model_revision='adaptive-fixture'").bind(a.id()).fetch_one(&db).await.unwrap();
    assert_eq!(inventory, (chunks + 20, 21));
    if !exact {
        let mut drained = false;
        for _ in 0..64 {
            if !store.maintain_search_vectors(false).await.unwrap() {
                drained = true;
                break;
            }
        }
        assert!(drained);
    }
    let filter = CachedSearchFilter {
        seen: exact.then_some(false),
        ..Default::default()
    };
    let hits = store
        .semantic_search_rows(&a, &filter, "adaptive-fixture", &axis(0))
        .await
        .unwrap();
    assert_eq!(
        hits.len(),
        21,
        "smaller ANN windows must expand past duplicate message chunks"
    );
    if exact {
        // The disposable F32 graph is only a read accelerator. Publishing it
        // must preserve the SQL-only exact result order and distances.
        while store.maintain_search_vectors(false).await.unwrap() {}
        let resident = store
            .semantic_search_rows(&a, &filter, "adaptive-fixture", &axis(0))
            .await
            .unwrap();
        assert_eq!(
            hits.iter()
                .map(|hit| (&hit.message.id, hit.distance))
                .collect::<Vec<_>>(),
            resident
                .iter()
                .map(|hit| (&hit.message.id, hit.distance))
                .collect::<Vec<_>>()
        );
        let phrases = vec!["searchable passage".to_owned()];
        let parallel = store
            .semantic_search_rows_with_phrases(&a, &filter, "adaptive-fixture", &axis(0), &phrases)
            .await
            .unwrap();
        let serial = store
            .constrain_search_phrases(&a, resident, &phrases)
            .await
            .unwrap();
        assert_eq!(
            parallel
                .iter()
                .map(|hit| (&hit.message.id, hit.distance))
                .collect::<Vec<_>>(),
            serial
                .iter()
                .map(|hit| (&hit.message.id, hit.distance))
                .collect::<Vec<_>>()
        );
        assert_eq!(parallel.len(), 21);
        assert!(store
            .semantic_search_rows_with_phrases(
                &a,
                &filter,
                "adaptive-fixture",
                &axis(0),
                &["absent phrase".into()]
            )
            .await
            .unwrap()
            .is_empty());
        let accelerator = VectorAccelerator::new(dir.path());
        catch_up(&accelerator, &db).await;
        let stored: Vec<(i64,String,Vec<u8>)> = sqlx::query_as("SELECT id,message_id,embedding FROM chunks WHERE account_id=? AND model_revision='adaptive-fixture' ORDER BY id").bind(a.id()).fetch_all(&db).await.unwrap();
        let ids: Vec<_> = stored.iter().map(|row| row.0).collect();
        assert_eq!(
            accelerator
                .cohort_vectors(a.id(), "adaptive-fixture", &ids)
                .await
                .unwrap()
                .unwrap(),
            stored
        );
        assert!(accelerator
            .cohort_vectors("another-account", "adaptive-fixture", &ids)
            .await
            .unwrap()
            .is_none());
        assert!(accelerator
            .cohort_vectors(a.id(), "adaptive-fixture", &[i64::MAX])
            .await
            .unwrap()
            .is_none());
        assert!(accelerator
            .cohort_vectors(a.id(), "adaptive-fixture", &vec![1; 4097])
            .await
            .is_err());
    }
    assert_eq!(hits[0].message.id, "adaptive-long");
    for i in 0..20 {
        assert!(hits
            .iter()
            .any(|hit| hit.message.id == format!("adaptive-{i:02}")));
    }
    if exact {
        let mut removed = hits[0].message.clone();
        removed.folder_ids = vec!["INBOX".into(), "Trash".into()];
        store.upsert_message(&a, removed).await.unwrap();
        // Leave the target stale deliberately. Concurrent phrase validation
        // cannot bypass live application membership/exclusion checks.
        let current = store
            .semantic_search_rows_with_phrases(
                &a,
                &filter,
                "adaptive-fixture",
                &axis(0),
                &["searchable passage".into()],
            )
            .await
            .unwrap();
        assert_eq!(current.len(), 20);
        assert!(current.iter().all(|hit| hit.message.id != "adaptive-long"));
    }
}
async fn embed(store: &Store, a: &AuthorizedAccount, revision: &str) {
    store.prepare_embeddings(a, revision, true).await.unwrap();
    store.discover_embeddings(a, revision).await.unwrap();
    for _ in 0..100 {
        let Some(turn) = store.claim_embedding(a, revision, true).await.unwrap() else {
            return;
        };
        let vector = axis(if turn.message_id.contains("second") {
            1
        } else {
            0
        });
        let passage = EmbeddedPassage {
            text: turn.text.clone(),
            byte_end: turn.text.len(),
            next_offset: turn.text.len(),
            vector: Some(vector),
        };
        assert!(store.commit_embedding(a, &turn, &passage).await.unwrap());
    }
    panic!("embedding fixture did not drain");
}
#[tokio::test]
async fn vectors_replay_atomic_snapshots_loss_corruption_and_other_consumers() {
    let (dir, mut store, _user, a, _) = fixture().await;
    configure_search_fixture(&mut store, None).await;
    // Startup is asynchronous. Complete migrations before testing delivery and
    // replay, so parallel fixture creation cannot consume the delivery deadline.
    store.maintain_search_vectors(false).await.unwrap();
    let mut second = message(&a, "second");
    second.starred = true;
    let messages = store
        .ingest_messages(&a, vec![message(&a, "first"), second], &[])
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    for m in &messages {
        store
            .put_search_body(&a, m, "durably retained body", "ready", Some(false))
            .await
            .unwrap();
    }
    store.flush_search_index(&a).await.unwrap();
    embed(&store, &a, "vector-fixture").await;
    let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap() else {
        unreachable!()
    };
    let accelerator = VectorAccelerator::new(dir.path());
    assert!(accelerator
        .candidates(a.id(), "vector-fixture", &axis(0), 10)
        .await
        .is_err());
    catch_up(&accelerator, &db).await;
    let hits = accelerator
        .candidates(a.id(), "vector-fixture", &axis(0), 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 4);
    assert!(hits[0].1.abs() < 1e-5);
    let filtered = accelerator
        .filtered_candidates(
            a.id(),
            "vector-fixture",
            &axis(0),
            10,
            Some(crate::search_backend::filter::CandidateFilter::from(
                &CachedSearchFilter {
                    starred: Some(true),
                    ..Default::default()
                },
            )),
        )
        .await
        .unwrap();
    assert_eq!(filtered.len(), 2);
    assert!(filtered
        .iter()
        .all(|(_, distance)| (*distance - 1.).abs() < 1e-5));
    assert!(accelerator
        .candidates("another-account", "vector-fixture", &axis(0), 10)
        .await
        .unwrap()
        .is_empty());
    assert!(accelerator
        .candidates(a.id(), "old-model", &axis(0), 10)
        .await
        .unwrap()
        .is_empty());
    assert!(accelerator
        .caught_up(&db, a.id(), "vector-fixture")
        .await
        .unwrap());
    sqlx::query("INSERT INTO embedding_changes(chunk_id,account_id,model_revision) VALUES(900000,'other-account','other-model')").execute(&db).await.unwrap();
    assert!(
        accelerator
            .caught_up(&db, a.id(), "vector-fixture")
            .await
            .unwrap(),
        "unrelated work must not degrade this account"
    );
    assert!(!accelerator
        .caught_up(&db, "other-account", "other-model")
        .await
        .unwrap());
    catch_up(&accelerator, &db).await;
    accelerator.snapshot(&db).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM embedding_changes")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    let identity: String = sqlx::query_scalar("SELECT value FROM index_identity WHERE id=1")
        .fetch_one(&db)
        .await
        .unwrap();
    let snapshot = dir
        .path()
        .join("search/vectors")
        .join(&identity)
        .join("manifest.json");
    let before: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&snapshot).unwrap()).unwrap();
    let mut first = messages[0].clone();
    first.starred = true;
    store.upsert_message(&a, first).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert!(!accelerator
        .caught_up(&db, a.id(), "vector-fixture")
        .await
        .unwrap());
    catch_up(&accelerator, &db).await;
    let filter = crate::search_backend::filter::CandidateFilter::from(&CachedSearchFilter {
        starred: Some(true),
        ..Default::default()
    });
    assert_eq!(
        accelerator
            .filtered_candidates(a.id(), "vector-fixture", &axis(0), 10, Some(filter.clone()))
            .await
            .unwrap()
            .len(),
        4
    );
    accelerator.snapshot(&db).await.unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&snapshot).unwrap()).unwrap();
    assert_eq!(
        before["graphs"], after["graphs"],
        "metadata replay must reuse unchanged graph files"
    );
    assert!(after["watermark"].as_i64() > before["watermark"].as_i64());
    // Restart uses the committed graph without requiring model or MIME data.
    let restarted = VectorAccelerator::new(dir.path());
    catch_up(&restarted, &db).await;
    assert_eq!(
        restarted
            .candidates(a.id(), "vector-fixture", &axis(0), 10)
            .await
            .unwrap(),
        hits
    );
    assert_eq!(
        restarted
            .filtered_candidates(a.id(), "vector-fixture", &axis(0), 10, Some(filter))
            .await
            .unwrap()
            .len(),
        4,
        "restart must rehydrate current metadata independently of reused graph files"
    );
    // A receipt can be replayed twice after an interrupted graph mutation.
    sqlx::query("UPDATE chunks SET embedding=embedding WHERE account_id=?")
        .bind(a.id())
        .execute(&db)
        .await
        .unwrap();
    catch_up(&accelerator, &db).await;
    assert_eq!(
        accelerator
            .candidates(a.id(), "vector-fixture", &axis(0), 10)
            .await
            .unwrap()
            .len(),
        4
    );
    // Delete while another consumer is behind, then prune after a snapshot.
    sqlx::query("DELETE FROM chunks WHERE account_id=? AND message_id=?")
        .bind(a.id())
        .bind(&messages[0].id)
        .execute(&db)
        .await
        .unwrap();
    catch_up(&accelerator, &db).await;
    accelerator.snapshot(&db).await.unwrap();
    assert!(
        !restarted
            .caught_up(&db, a.id(), "vector-fixture")
            .await
            .unwrap(),
        "a pruned replay floor fences stale graphs"
    );
    catch_up(&restarted, &db).await;
    assert_eq!(
        restarted
            .candidates(a.id(), "vector-fixture", &axis(0), 10)
            .await
            .unwrap()
            .len(),
        2
    );
    let identity: String = sqlx::query_scalar("SELECT value FROM index_identity WHERE id=1")
        .fetch_one(&db)
        .await
        .unwrap();
    let root = dir.path().join("search/vectors").join(&identity);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let graph = root.join(manifest["graphs"][0]["file"].as_str().unwrap());
    std::fs::write(&graph, b"corrupted graph").unwrap();
    let repaired = VectorAccelerator::new(dir.path());
    catch_up(&repaired, &db).await;
    assert_eq!(
        repaired
            .candidates(a.id(), "vector-fixture", &axis(0), 10)
            .await
            .unwrap()
            .len(),
        2
    );
    repaired.snapshot(&db).await.unwrap();
    // Lost accelerator files reconstruct exclusively from durable embeddings.
    std::fs::remove_dir_all(&root).unwrap();
    let lost = VectorAccelerator::new(dir.path());
    catch_up(&lost, &db).await;
    assert_eq!(
        lost.candidates(a.id(), "vector-fixture", &axis(0), 10)
            .await
            .unwrap()
            .len(),
        2
    );
    // Failed snapshot cannot acknowledge replay. Replace the directory with a
    // file to inject a portable filesystem failure, including on root test hosts.
    sqlx::query("UPDATE chunks SET embedding=embedding WHERE account_id=?")
        .bind(a.id())
        .execute(&db)
        .await
        .unwrap();
    catch_up(&lost, &db).await;
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM embedding_changes")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(pending > 0);
    std::fs::write(&root, b"not a directory").unwrap();
    assert!(lost.snapshot(&db).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM embedding_changes")
            .fetch_one(&db)
            .await
            .unwrap(),
        pending
    );
    std::fs::remove_file(&root).unwrap();
    lost.snapshot(&db).await.unwrap();
    // A stale in-memory snapshot must never advance a replacement database's
    // replay frontier, even if the epoch changes between maintenance turns.
    sqlx::query("UPDATE documents SET metadata=json_set(metadata,'$.unread',json('false')) WHERE account_id=? AND message_id='second'").bind(a.id()).execute(&db).await.unwrap();
    catch_up(&lost, &db).await;
    let floor: i64 = sqlx::query_scalar("SELECT floor FROM vector_replay WHERE id=1")
        .fetch_one(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE index_identity SET value=? WHERE id=1")
        .bind(Uuid::new_v4().to_string())
        .execute(&db)
        .await
        .unwrap();
    assert!(matches!(lost.snapshot(&db).await, Err(Error::Conflict)));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT floor FROM vector_replay WHERE id=1")
            .fetch_one(&db)
            .await
            .unwrap(),
        floor
    );
    let replacement = VectorAccelerator::new(dir.path());
    catch_up(&replacement, &db).await;
    assert_eq!(
        replacement
            .candidates(a.id(), "vector-fixture", &axis(0), 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

async fn retrieval(application: Option<&str>, url: Option<&str>) {
    let (dir, _, _, _, input) = fixture().await;
    let root = dir.path().join("retrieval");
    let mut store = Store::open_with_database(&root, [31; 32], 1024 * 1024 * 1024, application)
        .await
        .unwrap();
    let sqlite_url = format!("sqlite:{}", root.join("selected-search.sqlite").display());
    configure_search_fixture(&mut store, Some(url.unwrap_or(&sqlite_url))).await;
    let user = store
        .user("fixture", &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let email = input.email.clone();
    let other = store
        .connect(
            &user,
            &ConnectAccount {
                email: "other-vector@example.invalid".into(),
                ..input
            },
        )
        .await
        .unwrap();
    let other = store.authorize(&user, &other.id).await.unwrap();
    for account in [&a, &other] {
        let messages = store
            .ingest_messages(
                account,
                vec![message(account, "first"), message(account, "second")],
                &[],
            )
            .await
            .unwrap();
        store.index_search_headers(account).await.unwrap();
        for m in &messages {
            store
                .put_search_body(account, m, "durably retained body", "ready", Some(false))
                .await
                .unwrap();
        }
        store.flush_search_index(account).await.unwrap();
        embed(&store, account, "retrieval-model").await;
    }
    for _ in 0..100 {
        if !store.maintain_search_vectors(false).await.unwrap() {
            break;
        }
    }
    let rows = store
        .semantic_search_rows(
            &a,
            &CachedSearchFilter::default(),
            "retrieval-model",
            &axis(0),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].message.id.contains("first"));
    assert!(rows[0].distance.abs() < 1e-5);
    assert!(rows.iter().all(|r| r.message.account_id == a.id()));
    assert!(rows.iter().all(|r| !r.excerpt.is_empty()));
    assert!(store
        .semantic_search_rows(
            &a,
            &CachedSearchFilter {
                subject: Some("absent"),
                ..Default::default()
            },
            "retrieval-model",
            &axis(0)
        )
        .await
        .unwrap()
        .is_empty());
    assert!(store
        .semantic_search_rows(
            &a,
            &CachedSearchFilter {
                seen: Some(true),
                ..Default::default()
            },
            "retrieval-model",
            &axis(0)
        )
        .await
        .unwrap()
        .is_empty());
    // Unacknowledged source changes and alternate Trash membership invalidate
    // old accelerator hits immediately, before target delivery or graph replay.
    let mut first = rows[0].message.clone();
    first.folder_ids.push("Trash".into());
    store.upsert_message(&a, first).await.unwrap();
    let rows = store
        .semantic_search_rows(
            &a,
            &CachedSearchFilter::default(),
            "retrieval-model",
            &axis(0),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].message.id.contains("second"));
    store.remove_account(&a, &email).await.unwrap();
    store
        .remove_account(&other, "other-vector@example.invalid")
        .await
        .unwrap();
    assert!(store
        .semantic_search_rows(
            &a,
            &CachedSearchFilter::default(),
            "retrieval-model",
            &axis(0)
        )
        .await
        .is_err());
}
#[tokio::test]
async fn sqlite_semantic_retrieval_validates_current_source() {
    retrieval(None, None).await;
}
#[tokio::test]
#[ignore = "requires DAKIA_TEST_SEARCH_DATABASE_URL with pgvector"]
async fn postgres_semantic_retrieval_validates_current_source() {
    let url = std::env::var("DAKIA_TEST_SEARCH_DATABASE_URL").unwrap();
    let application = std::env::var("DAKIA_TEST_APPLICATION_DATABASE_URL").unwrap();
    retrieval(None, Some(&url)).await;
    retrieval(Some(&application), None).await;
    retrieval(Some(&application), Some(&url)).await;
}
