use super::*;
use crate::search_backend::SearchDatabase;
const MODEL: &str = "filtered-fixture-v1";

#[tokio::test]
async fn attachment_filter_applies_to_keyword_vectors_and_metadata_updates() {
    let (dir, mut store, _user, a, _) = fixture().await;
    store.configure_search_database(None).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let attached = store
        .upsert_message(&a, message(&a, "attached"))
        .await
        .unwrap();
    let plain = store
        .upsert_message(&a, message(&a, "plain"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_bodies(
            &a,
            &[
                (&attached, "body", "ready", Some(true)),
                (&plain, "body", "ready", Some(false)),
            ],
        )
        .await
        .unwrap();
    while store.flush_search_index(&a).await.unwrap() {}
    let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap() else {
        unreachable!()
    };
    let mut vector = vec![0f32; 384];
    vector[0] = 1.;
    let bytes = vector
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT account_id,message_id,source,?,0,'body',?,content_version,1 FROM documents WHERE account_id=?")
        .bind(MODEL).bind(bytes).bind(a.id()).execute(&db).await.unwrap();
    drain(&store).await;
    let filter = || CachedSearchFilter {
        has_attachment: true,
        ..Default::default()
    };
    check(&store, &a, filter(), &["attached"]).await;
    let rows = store.full_text_rows(&a, &filter(), None).await.unwrap();
    assert_eq!(
        rows.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["attached"]
    );
    store
        .put_search_body(&a, &plain, "body", "ready", Some(true))
        .await
        .unwrap();
    while store.flush_search_index(&a).await.unwrap() {}
    drain(&store).await;
    check(&store, &a, filter(), &["attached", "plain"]).await;
    store
        .put_search_body(&a, &attached, "body", "ready", Some(false))
        .await
        .unwrap();
    while store.flush_search_index(&a).await.unwrap() {}
    drain(&store).await;
    check(&store, &a, filter(), &["plain"]).await;
    let live = store
        .current_indexed_search_rows(&a, &[attached.id, plain.id], &filter())
        .await
        .unwrap();
    assert_eq!(
        live.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["plain"]
    );
    assert_eq!(
        store
            .full_text_rows(&a, &filter(), None)
            .await
            .unwrap()
            .len(),
        1
    );
}

async fn drain(store: &Store) {
    for _ in 0..128 {
        if !store.maintain_search_vectors(true).await.unwrap() {
            return;
        }
    }
    panic!("accelerator did not catch up");
}
async fn check(
    store: &Store,
    a: &AuthorizedAccount,
    filter: CachedSearchFilter<'_>,
    expected: &[&str],
) {
    let mut query = vec![0.; 384];
    query[0] = 1.;
    let rows = store
        .semantic_search_rows(a, &filter, MODEL, &query)
        .await
        .unwrap_or_else(|error| panic!("{filter:?}: {error:?}"));
    let mut ids: Vec<_> = rows.iter().map(|m| m.message.id.as_str()).collect();
    ids.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(ids, expected, "filter: {filter:?}");
}
async fn contract(application: Option<&str>, search: Option<&str>) {
    let (dir, mut store, mut user, mut a, input) = fixture().await;
    if application.is_some() {
        store = Store::open_with_database(dir.path(), [31; 32], 1024 * 1024 * 1024, application)
            .await
            .unwrap();
        user = store
            .user("filter-fixture", &Uuid::new_v4().to_string())
            .await
            .unwrap();
        let account = store.connect(&user, &input).await.unwrap();
        a = store.authorize(&user, &account.id).await.unwrap();
    }
    let sqlite = format!(
        "sqlite:{}",
        dir.path().join("filtered-search.sqlite").display()
    );
    let url = Some(search.unwrap_or(&sqlite));
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let mut records = Vec::new();
    for i in 0..1051 {
        let mut m = message(&a, &format!("filter-{i:04}"));
        m.category = Some("newsletters".into());
        m.received_date = Some("2026-10-29".into());
        if i == 0 || i == 1050 {
            m.thread_id = "shared-filter-thread".into();
        }
        if i == 1050 {
            m.sender.email = "rare-sender@example.invalid".into();
            m.to = vec![Address {
                email: "rare-recipient@example.invalid".into(),
                name: None,
            }];
            m.subject = "Reserved semantic %_!\" ÄÖ東京".into();
            m.folder_ids = vec!["Archive/Rare".into()];
            m.unread = false;
            m.starred = true;
            m.category = Some("people".into());
            m.classification = Some(Classification {
                kind: InboxKind::Personal,
                status: CategoryStatus::Ready,
                types: vec![DetectedType::Receipt, DetectedType::Invoice],
                ..Default::default()
            });
            m.received_date = Some("2026-10-30".into());
        }
        records.push(m);
    }
    let mut stored = Vec::new();
    for batch in records.chunks(200) {
        stored.extend(
            store
                .ingest_messages(&a, batch.to_vec(), &[])
                .await
                .unwrap(),
        );
        store.index_search_headers(&a).await.unwrap();
    }
    for batch in stored.chunks(200) {
        store
            .put_search_bodies(
                &a,
                &batch
                    .iter()
                    .map(|m| (m, "semantic passage", "ready", Some(false)))
                    .collect::<Vec<_>>(),
            )
            .await
            .unwrap();
    }
    for _ in 0..16 {
        if !store.flush_search_index(&a).await.unwrap() {
            break;
        }
    }
    let target = SearchDatabase::open(dir.path(), url).await.unwrap();
    let mut near = vec![0f32; 384];
    near[0] = 1.;
    let mut far = vec![0f32; 384];
    far[1] = 1.;
    match &target {
        SearchDatabase::Sqlite(db) => {
            let bytes = |v: &[f32]| v.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
            sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT account_id,message_id,source,'filtered-fixture-v1',0,'semantic passage',CASE WHEN message_id='filter-1050' THEN ? ELSE ? END,content_version,1 FROM documents WHERE account_id=?")
                .bind(bytes(&far)).bind(bytes(&near)).bind(a.id()).execute(db).await.unwrap();
            sqlx::query("WITH RECURSIVE numbers(n) AS(VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<20001) INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT account_id,message_id,source,model_revision,n,text,embedding,content_version,field FROM chunks CROSS JOIN numbers WHERE account_id=? AND message_id='filter-0000' AND model_revision=? AND ordinal=0")
                .bind(a.id()).bind(MODEL).execute(db).await.unwrap();
        }
        SearchDatabase::Postgres(db) => {
            sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT account_id,message_id,source,'filtered-fixture-v1',0,'semantic passage',CASE WHEN message_id='filter-1050' THEN $2::text::vector ELSE $3::text::vector END,content_version,1 FROM documents WHERE account_id=$1")
                .bind(a.id()).bind(serde_json::to_string(&far).unwrap()).bind(serde_json::to_string(&near).unwrap()).execute(db).await.unwrap();
            // More closer passages than the ANN scan budget must not hide the only eligible message.
            sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field) SELECT account_id,message_id,source,model_revision,n,text,embedding,content_version,field FROM chunks CROSS JOIN generate_series(1,20001) n WHERE account_id=$1 AND message_id='filter-0000' AND model_revision=$2 AND ordinal=0")
                .bind(a.id()).bind(MODEL).execute(db).await.unwrap();
        }
    }
    drain(&store).await;
    let unfiltered = store
        .semantic_search_rows(&a, &CachedSearchFilter::default(), MODEL, &near)
        .await
        .unwrap();
    assert_eq!(
        unfiltered.len(),
        200,
        "one long email must not starve other messages"
    );
    assert!(
        !unfiltered.iter().any(|m| m.message.id == "filter-1050"),
        "rare fixture must sit beyond the unfiltered candidate limit"
    );
    let rare = ["filter-1050"];
    for classification in [
        ClassificationFilter {
            email_type: Some(DetectedType::Receipt),
            ..Default::default()
        },
        ClassificationFilter {
            transactions: true,
            ..Default::default()
        },
    ] {
        check(
            &store,
            &a,
            CachedSearchFilter {
                classification: classification.clone(),
                ..Default::default()
            },
            &rare,
        )
        .await;
        let rows = store
            .full_text_ranked_candidates(
                &a,
                &CachedSearchFilter {
                    classification,
                    text: Some("semantic"),
                    ..Default::default()
                },
                None,
                51,
            )
            .await
            .unwrap();
        assert_eq!(
            rows.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(),
            rare
        );
    }

    check(
        &store,
        &a,
        CachedSearchFilter {
            starred: Some(true),
            ..Default::default()
        },
        &rare,
    )
    .await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            seen: Some(true),
            ..Default::default()
        },
        &rare,
    )
    .await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            from: Some("rare-sender"),
            ..Default::default()
        },
        &rare,
    )
    .await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            to: Some("rare-recipient"),
            ..Default::default()
        },
        &rare,
    )
    .await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            subject: Some("Reserved"),
            ..Default::default()
        },
        &rare,
    )
    .await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            category: Some("people"),
            ..Default::default()
        },
        &rare,
    )
    .await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            since: Some("2026-10-30"),
            before: Some("2026-10-31"),
            ..Default::default()
        },
        &rare,
    )
    .await;
    let folders = vec!["Archive/Rare".into()];
    check(
        &store,
        &a,
        CachedSearchFilter {
            folders: Some(&folders),
            ..Default::default()
        },
        &rare,
    )
    .await;
    let current = store.message(&a, "filter-1050").await.unwrap();
    store
        .operation(
            &a,
            Operation {
                id: "filter-snooze".into(),
                account_id: a.id().into(),
                target_id: current.id.clone(),
                expected_revision: current.revision,
                change: Change::Snooze {
                    until: "2099-01-01T00:00:00Z".into(),
                },
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    drain(&store).await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            snoozed_only: true,
            ..Default::default()
        },
        &["filter-0000", "filter-1050"],
    )
    .await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            starred: Some(true),
            hide_snoozed: true,
            ..Default::default()
        },
        &[],
    )
    .await;
    let mut changed = store.message(&a, "filter-1050").await.unwrap();
    changed.starred = false;
    store.upsert_message(&a, changed).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    if search.is_none() {
        assert!(
            matches!(
                store
                    .semantic_search_rows(&a, &CachedSearchFilter::default(), MODEL, &near)
                    .await,
                Err(Error::Conflict)
            ),
            "unreplayed metadata must fence complete vector results"
        );
    }
    drain(&store).await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            starred: Some(true),
            ..Default::default()
        },
        &[],
    )
    .await;
    let current = store.message(&a, "filter-1050").await.unwrap();
    store
        .operation(
            &a,
            Operation {
                id: "filter-wake".into(),
                account_id: a.id().into(),
                target_id: current.id.clone(),
                expected_revision: current.revision,
                change: Change::Snooze {
                    until: "2020-01-01T00:00:00Z".into(),
                },
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    store.wake_snoozes(&a).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    drain(&store).await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            snoozed_only: true,
            ..Default::default()
        },
        &[],
    )
    .await;

    for value in ["%_!\"", "\" ÄÖ", "ÄÖ東京", "東京"] {
        let filter = CachedSearchFilter {
            subject: Some(value),
            ..Default::default()
        };
        assert_eq!(
            store.full_text_rows(&a, &filter, None).await.unwrap().len(),
            1
        );
        check(&store, &a, filter, &rare).await;
    }
    for value in ["%X!", "Reserved absent", "äö大阪"] {
        let filter = CachedSearchFilter {
            subject: Some(value),
            ..Default::default()
        };
        assert!(store
            .full_text_rows(&a, &filter, None)
            .await
            .unwrap()
            .is_empty());
        check(&store, &a, filter, &[]).await;
    }
    // Verify a usable index path independently of small-fixture planner costs.
    match &target {
        SearchDatabase::Sqlite(db) => {
            let rows = sqlx::query("EXPLAIN QUERY PLAN SELECT rowid FROM metadata_terms WHERE metadata_terms MATCH 'subject:\"res\"'").fetch_all(db).await.unwrap();
            use sqlx::Row;
            assert!(rows.iter().any(|row| row
                .get::<String, _>("detail")
                .contains("VIRTUAL TABLE INDEX")));
            sqlx::query(
                "INSERT INTO metadata_terms(metadata_terms,rank) VALUES('integrity-check',1)",
            )
            .execute(db)
            .await
            .unwrap();
        }
        SearchDatabase::Postgres(db) => {
            let mut tx = db.begin().await.unwrap();
            sqlx::query("SET LOCAL enable_seqscan=off")
                .execute(&mut *tx)
                .await
                .unwrap();
            let plan: Vec<String> = sqlx::query_scalar(
                "EXPLAIN SELECT message_id FROM documents WHERE subject LIKE '%reserved%'",
            )
            .fetch_all(&mut *tx)
            .await
            .unwrap();
            assert!(
                plan.iter()
                    .any(|line| line.contains("documents_subject_typos")),
                "{plan:?}"
            );
            tx.commit().await.unwrap();
        }
    }
    drop(store);
    let mut store =
        Store::open_with_database(dir.path(), [31; 32], 1024 * 1024 * 1024, application)
            .await
            .unwrap();
    let a = store.authorize(&user, a.id()).await.unwrap();
    store.configure_search_database(url).await.unwrap();
    drain(&store).await;
    check(
        &store,
        &a,
        CachedSearchFilter {
            from: Some("rare-sender"),
            hide_snoozed: true,
            ..Default::default()
        },
        &rare,
    )
    .await;
    let mut renamed = store.message(&a, "filter-1050").await.unwrap();
    renamed.subject = "Replacement_%! 東京".into();
    store.upsert_message(&a, renamed).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    for (value, expected) in [("Reserved", 0), ("Replacement_%! 東京", 1)] {
        let rows = store
            .full_text_rows(
                &a,
                &CachedSearchFilter {
                    subject: Some(value),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), expected);
    }
    store.remove_account(&a, &input.email).await.unwrap();
    if let SearchDatabase::Sqlite(db) = &target {
        sqlx::query("INSERT INTO metadata_terms(metadata_terms,rank) VALUES('integrity-check',1)")
            .execute(db)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM metadata_terms WHERE metadata_terms MATCH 'subject:\"rep\"'",
        )
        .fetch_one(db)
        .await
        .unwrap();
        assert_eq!(count, 0, "account removal must remove metadata postings");
    }
}
#[tokio::test]
async fn sqlite_filters_find_far_neighbors_and_replay_thread_metadata_after_restart() {
    contract(None, None).await;
}
#[tokio::test]
#[ignore = "requires isolated pgvector PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
async fn postgres_filters_find_far_neighbors_and_replay_thread_metadata_after_restart() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap();
    contract(None, Some(&url)).await;
    contract(Some(&url), None).await;
    contract(Some(&url), Some(&url)).await;
}

async fn attachment_provenance(application: Option<&str>, search: Option<&str>) {
    let (dir, mut store, _user, mut a, input) = fixture().await;
    if application.is_some() {
        store = Store::open_with_database(dir.path(), [31; 32], 1024 * 1024 * 1024, application)
            .await
            .unwrap();
        let user = store
            .user("attachment-fixture", &Uuid::new_v4().to_string())
            .await
            .unwrap();
        let account = store.connect(&user, &input).await.unwrap();
        a = store.authorize(&user, &account.id).await.unwrap();
    }
    let sqlite = format!(
        "sqlite:{}",
        dir.path().join("attachment-search.sqlite").display()
    );
    let url = Some(search.unwrap_or(&sqlite));
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let m = store
        .upsert_message(&a, message(&a, "attachment-provenance"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    let first = "first.txt\nshort\n";
    let second = "second.txt\ni\u{307}stanbul café delivery details in the second file\n";
    let text = format!("{first}{second}");
    let details = serde_json::json!([
        {"part":0,"filename":"first.txt","state":"ready","byte_start":0,"byte_end":first.len()},
        {"part":1,"filename":"second.txt","state":"ready","byte_start":first.len(),"byte_end":text.len()}
    ]).to_string();
    store
        .put_search_records(
            &a,
            &[SearchContent {
                has_attachment: Some(true),
                message: &m,
                text: "readable prefix",
                state: "oversized",
                participants: "reply@example.invalid",
                recipients: "carbon@example.invalid",
                attachments: &text,
                attachment_details: &details,
            }],
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let lexical = store
        .full_text_rows(
            &a,
            &CachedSearchFilter {
                text: Some("readable prefix"),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        lexical.len(),
        1,
        "retain readable text alongside a resource gap"
    );
    let target = SearchDatabase::open(dir.path(), url).await.unwrap();
    let mut vector = vec![0f32; 384];
    vector[0] = 1.;
    // Several matching passages must materialize only one payload per message.
    for ordinal in 0..4i64 {
        match &target {
            SearchDatabase::Sqlite(db) => {
                let bytes: Vec<_> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
                sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field,byte_start,byte_end) SELECT account_id,message_id,source,?, ?,attachment_text,?,content_version,2,0,? FROM documents WHERE account_id=? AND message_id=?")
                    .bind(MODEL).bind(ordinal).bind(bytes).bind(text.len() as i64).bind(a.id()).bind(&m.id).execute(db).await.unwrap();
            }
            SearchDatabase::Postgres(db) => {
                sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field,byte_start,byte_end) SELECT account_id,message_id,source,$1,$2,attachment_text,$3::text::vector,content_version,2,0,$4 FROM documents WHERE account_id=$5 AND message_id=$6")
                    .bind(MODEL).bind(ordinal).bind(serde_json::to_string(&vector).unwrap()).bind(text.len() as i64).bind(a.id()).bind(&m.id).execute(db).await.unwrap();
            }
        }
    }
    drain(&store).await;
    let rows = store
        .semantic_search_rows(&a, &CachedSearchFilter::default(), MODEL, &vector)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].attachment.as_deref(), Some("second.txt"));
    assert_eq!(rows[0].excerpt, second);
    let mut changed = m.clone();
    changed.starred = true;
    store.upsert_message(&a, changed).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    drain(&store).await;
    let rows = store
        .semantic_search_rows(
            &a,
            &CachedSearchFilter {
                starred: Some(true),
                ..Default::default()
            },
            MODEL,
            &vector,
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].attachment.as_deref(), Some("second.txt"));
    store.remove_account(&a, &input.email).await.unwrap();
}
#[tokio::test]
async fn sqlite_attachment_provenance_survives_content_gaps_and_metadata_patches() {
    attachment_provenance(None, None).await;
}
#[tokio::test]
#[ignore = "requires isolated pgvector PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
async fn postgres_attachment_provenance_survives_content_gaps_and_metadata_patches() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap();
    attachment_provenance(None, Some(&url)).await;
    attachment_provenance(Some(&url), None).await;
    attachment_provenance(Some(&url), Some(&url)).await;
}

#[tokio::test]
async fn metadata_only_search_survives_loss_of_the_substring_accelerator() {
    let (dir, mut store, _user, a, _) = fixture().await;
    store.configure_search_database(None).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let mut incoming = message(&a, "metadata-fallback");
    incoming.sender.email = "filter@example.invalid".into();
    incoming.to = vec![Address {
        email: "carbon@example.invalid".into(),
        name: None,
    }];
    incoming.subject = "東京 metadata".into();
    let saved = store.upsert_message(&a, incoming).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &saved, "readable body", "ready", Some(false))
        .await
        .unwrap();
    while store.flush_search_index(&a).await.unwrap() {}
    let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap() else {
        unreachable!()
    };
    sqlx::query("DROP TABLE metadata_terms")
        .execute(&db)
        .await
        .unwrap();
    let rows = store
        .full_text_rows(
            &a,
            &CachedSearchFilter {
                from: Some("filter@"),
                to: Some("carbon@"),
                subject: Some("東京"),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "metadata-fallback");
    assert!(store
        .full_text_rows(
            &a,
            &CachedSearchFilter {
                subject: Some("大阪"),
                ..Default::default()
            },
            None
        )
        .await
        .unwrap()
        .is_empty());
}
