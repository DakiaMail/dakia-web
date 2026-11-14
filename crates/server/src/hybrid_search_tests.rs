use super::*;
use dakia_storage::EmbeddedPassage;
use std::collections::BTreeSet;
async fn fixture_backend(
    count: usize,
    real: bool,
    application: Option<&str>,
    search: Option<&str>,
) -> (
    tempfile::TempDir,
    crate::App,
    String,
    AuthorizedAccount,
    Arc<Controlled>,
) {
    let (dir, mut app, mut user, mut a, mut provider) = controlled().await;
    if application.is_some() {
        let input = app.store.credential(&a).await.unwrap();
        app.store = dakia_storage::Store::open_with_database(
            dir.path().join("hybrid-app"),
            [27; 32],
            1024 * 1024 * 1024,
            application,
        )
        .await
        .unwrap();
        user = app
            .store
            .user("fixture", &uuid::Uuid::new_v4().to_string())
            .await
            .unwrap();
        let account = app.store.connect(&user, &input).await.unwrap();
        a = app.store.authorize(&user, &account.id).await.unwrap();
        let base = Arc::new(InboxProvider {
            account: a.id().into(),
            count: 2,
            delay: 0,
            scans: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        });
        let page = base
            .scan("INBOX", &p::Search::default(), None, 50)
            .await
            .unwrap();
        base.scans.store(0, Ordering::SeqCst);
        provider = Arc::new(Controlled {
            base,
            page: Mutex::new(page),
            failure: Mutex::new(None),
            head_calls: AtomicUsize::new(0),
            fetch_calls: AtomicUsize::new(0),
            recovery_calls: AtomicUsize::new(0),
            recovered: Mutex::new(None),
            raw: Mutex::new(MIME.to_vec()),
            mutations: Mutex::new(vec![]),
            stale_inbox: AtomicBool::new(false),
            search_delay: Mutex::new(Duration::ZERO),
        });
        app.mail
            .providers
            .lock()
            .await
            .insert(a.id().into(), provider.clone());
    }
    let sqlite = format!(
        "sqlite:{}",
        dir.path().join("hybrid-search.sqlite").display()
    );
    let search = Some(search.unwrap_or(&sqlite));
    let revision = dakia_inference::embedding::search_model_revision();
    app.store.require_search_embeddings(&revision).unwrap();
    app.store.configure_search_database(search).await.unwrap();
    app.store.maintain_search_vectors(false).await.unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../inference/assets/multilingual-e5-small");
    let mut actual = if real {
        Some(dakia_inference::embedding::LocalSearchEmbedder::from_dir(&path).unwrap())
    } else {
        None
    };
    if real {
        app.mail = app.mail.with_search_model(path);
    } else {
        let mut vector = [0.; 384];
        vector[0] = 1.;
        app.mail.search_queries = crate::search_query_model::QueryModel::fixture(vector);
    }
    let mut messages = vec![];
    for index in 0..count {
        let id = format!("hybrid-{index:03}");
        let subject = if real {
            [
                "Package delivery",
                "Shipping update",
                "Garden notes",
                "Your Lisbon stay is confirmed",
                "Lisbon trip: accommodation details",
                "Everything for your arrival",
                "Lisbon restaurant recommendations",
                "Your Barcelona stay is confirmed",
                "Office network maintenance",
            ][index]
        } else {
            "Package"
        };
        let message:Message=serde_json::from_value(serde_json::json!({"id":id,"account_id":a.id(),"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":"Maya"},"to":[],"subject":subject,"preview":"","received_at":"2020-01-01T00:00:00.000Z","received_date":"2020-01-01","unread":index%2==0,"starred":false,"category":null,"category_status":"pending","revision":1})).unwrap();
        messages.push(message);
    }
    let mut stored = vec![];
    for batch in messages.chunks(200) {
        stored.extend(
            app.store
                .ingest_messages(&a, batch.to_vec(), &[])
                .await
                .unwrap(),
        );
        app.store.index_search_headers(&a).await.unwrap();
    }
    // A completed offline index must stay complete when folder discovery ages.
    app.store.put_provider_document(&a,"sync:folders",&serde_json::json!({"folders":[{"name":"INBOX","selectable":true,"attributes":[]}],"index":0,"refreshed":1}).to_string()).await.unwrap();
    let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    for (index, batch) in stored.chunks(200).enumerate() {
        let last = (index + 1) * 200 >= stored.len();
        let page = SearchPage {
            rows: batch.to_vec(),
            continuation: (!last).then_some(format!("fixture-page-{index}")),
            coverage: vec![AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec!["INBOX".into()],
                state: CoverageState::Complete,
                error: None,
                inbox: None,
            }],
            classification: ClassificationCoverage {
                classified: 0,
                total: count as u32,
            },
            pending: !last,
            counts_exact: false,
            cache_preview: None,
            index_page: None,
            search: None,
            supplement: None,
        };
        app.store
            .record_scan(&a, "INBOX", generation, &page)
            .await
            .unwrap();
    }
    for batch in stored.chunks(200) {
        let records: Vec<_> = batch
            .iter()
            .map(|m| {
                let text = if real {
                    match m.id.as_str() {
                        "hybrid-000" => "Your package delivery receipt is attached.",
                        "hybrid-001" => {
                            "The parcel will arrive tomorrow. Track this shipment with the courier."
                        }
                        "hybrid-003" => "Alfama House. Your Lisbon stay is confirmed. Check-in details and address: Rua dos Remedios 42, Lisbon, Portugal. Your room is ready from 15:00.",
                        "hybrid-004" => "The guesthouse in Lisbon is booked at Alfama House, Rua dos Remedios 42. Here are the accommodation details for our trip.",
                        "hybrid-005" => "Welcome to Alfama House in Lisbon. Your room, arrival time and how to find us: Rua dos Remedios 42. Check in after 15:00.",
                        "hybrid-006" => "For dinner in Lisbon, try the seafood restaurant near the river. This is a restaurant recommendation, with no accommodation booking.",
                        "hybrid-007" => "Your hotel in Barcelona is booked. The address is Carrer de Mallorca 100, Barcelona, Spain. Check in at 15:00.",
                        "hybrid-008" => "The office network will be offline on Saturday for router maintenance.",
                        _ => "Plants and roses require sunlight and watering every day.",
                    }
                } else {
                    "Historical archive body"
                };
                dakia_storage::SearchContent {
                    message: m,
                    text,
                    state: "ready",
                    participants: "",
                    recipients: "",
                    attachments: "",
                    attachment_details: "[]",
                }
            })
            .collect();
        app.store.put_search_records(&a, &records).await.unwrap();
    }
    for _ in 0..10 {
        if !app.store.flush_search_index(&a).await.unwrap() {
            break;
        }
    }
    app.store
        .prepare_embeddings(&a, &revision, true)
        .await
        .unwrap();
    for _ in 0..20 {
        if !app.store.discover_embeddings(&a, &revision).await.unwrap() {
            break;
        }
    }
    for _ in 0..count * 4 + 10 {
        let Some(turn) = app
            .store
            .claim_embedding(&a, &revision, true)
            .await
            .unwrap()
        else {
            break;
        };
        let vector = if let Some(model) = actual.as_mut() {
            model
                .embed_passages(&[&turn.text])
                .unwrap()
                .remove(0)
                .to_vec()
        } else {
            let mut v = vec![0.; 384];
            v[usize::from(turn.message_id != "hybrid-000")] = 1.;
            v
        };
        let passage = EmbeddedPassage {
            text: turn.text.clone(),
            byte_end: turn.text.len(),
            next_offset: turn.text.len(),
            vector: Some(vector),
        };
        app.store
            .commit_embedding(&a, &turn, &passage)
            .await
            .unwrap();
    }
    for _ in 0..100 {
        if !app.store.advance_search_lexical(&a, true).await.unwrap() {
            break;
        }
    }
    for _ in 0..10 {
        if !app.store.flush_embedding_receipts(&a).await.unwrap() {
            break;
        }
    }
    for _ in 0..100 {
        if !app.store.maintain_search_vectors(true).await.unwrap() {
            break;
        }
    }
    assert_eq!(
        app.store.text_coverage(&a).await.unwrap().processed,
        count as i64
    );
    for _ in 0..2000 {
        if app.mail.search_queries.state() == crate::search_query_model::QueryModelState::Ready {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(
        app.mail.search_queries.state(),
        crate::search_query_model::QueryModelState::Ready
    );
    (dir, app, user, a, provider)
}
#[tokio::test]
async fn hybrid_frozen_pages_preserve_rank_dedup_and_live_exclusions() {
    let (_dir, app, user, a, provider) = fixture_backend(550, false, None, None).await;
    let before = provider.base.scans.load(Ordering::SeqCst);
    let mut page = run(&app, &user, "packgae", a.id(), None).await.unwrap();
    assert!(!page.pending, "caught-up hybrid search must be complete");
    assert!(page.search.as_ref().unwrap().degradation.is_empty());
    let saved = app
        .store
        .load_search_cursor(&user, page.continuation.as_ref().unwrap())
        .await
        .unwrap();
    let mut old: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        old["spelling"][a.id()],
        serde_json::json!([["packgae", "package"]]),
        "continuations must freeze spelling alternatives"
    );
    for version in [4, 5, 6] {
        old["version"] = serde_json::json!(version);
        let old = app
            .store
            .save_search_cursor(&user, &old.to_string())
            .await
            .unwrap();
        let rejected = run(&app, &user, "packgae", a.id(), Some(old))
            .await
            .unwrap_err();
        assert_eq!(
            (rejected.0, rejected.1),
            (axum::http::StatusCode::CONFLICT, "refetch_required")
        );
    }
    let mut excluded = page.rows.last().unwrap().clone();
    // Pick a frozen result that has not yet been presented.
    excluded.id = "hybrid-100".into();
    excluded.provider_id = excluded.id.clone();
    excluded.thread_id = excluded.id.clone();
    excluded.folder_ids = vec!["INBOX".into(), "Trash".into()];
    app.store.upsert_message(&a, excluded).await.unwrap();
    app.store.index_search_headers(&a).await.unwrap();
    app.store.flush_search_index(&a).await.unwrap();
    let mut seen = BTreeSet::new();
    let mut order = vec![];
    let mut previous = f64::INFINITY;
    let mut count = 0;
    loop {
        let details = page.search.as_ref().unwrap();
        for row in &page.rows {
            order.push(row.id.clone());
            assert!(seen.insert(row.id.clone()), "duplicate {}", row.id);
            assert_ne!(row.id, "hybrid-100");
            let score = details
                .matches
                .iter()
                .find(|m| m.message_id == row.id)
                .unwrap()
                .score;
            assert!(score <= previous, "saved ranking changed between pages");
            previous = score;
        }
        let Some(token) = page.continuation else {
            break;
        };
        count += 1;
        assert!(count < 30, "finite lexical tail did not drain");
        page = run(&app, &user, "packgae", a.id(), Some(token))
            .await
            .unwrap();
    }
    assert_eq!(seen.len(), 549);
    assert!(
        order.iter().position(|id| id == "hybrid-000").unwrap() < 512,
        "semantic evidence must elevate a lexical-tail hit"
    );
    assert_eq!(
        provider.base.scans.load(Ordering::SeqCst),
        before,
        "healthy searches must never wait for providers"
    );
}
#[tokio::test]
async fn hybrid_phrases_are_strict_and_model_failure_retains_lexical_matches() {
    let (_dir, mut app, user, a, _provider) = fixture_backend(3, false, None, None).await;
    let page = run(&app, &user, r#""package delivery""#, a.id(), None)
        .await
        .unwrap();
    assert!(
        page.rows.is_empty(),
        "semantic similarity must not weaken a quoted phrase"
    );
    app.mail.search_queries = crate::search_query_model::QueryModel::configured(
        std::path::PathBuf::from("/nonexistent/dakia-fixture-model"),
    );
    let page = run(&app, &user, "package", a.id(), None).await.unwrap();
    assert_eq!(page.rows.len(), 3);
    assert!(page.pending);
    assert!(page
        .search
        .unwrap()
        .degradation
        .iter()
        .any(|d| d.code == "semantic_search_unavailable"));
}
#[tokio::test]
#[ignore = "requires pinned CPU E5 assets; scripts/fetch-search-model.py"]
async fn real_hybrid_search_returns_meaning_and_preserves_quoted_constraints() {
    let (_dir, app, user, a, provider) = fixture_backend(3, true, None, None).await;
    let page = run(&app, &user, "package delivery", a.id(), None)
        .await
        .unwrap();
    assert!(!page.pending);
    assert_eq!(page.rows[0].id, "hybrid-000");
    assert!(
        page.rows.iter().any(|m| m.id == "hybrid-001"),
        "parcel passage has no exact package/delivery terms"
    );
    let phrase = run(&app, &user, r#""package delivery""#, a.id(), None)
        .await
        .unwrap();
    assert_eq!(
        phrase
            .rows
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["hybrid-000"]
    );
    let misspelled = run(&app, &user, "pakcage delivrey", a.id(), None)
        .await
        .unwrap();
    assert!(!misspelled.pending);
    assert_eq!(misspelled.rows[0].id, "hybrid-000");
    assert!(
        misspelled.search.unwrap().matches[0].score > 0.03,
        "local spelling and actual semantic retrieval must both contribute"
    );
    for query in [
        "Millal minu pakk kohale jõuab?",
        "میرا پارسل کب پہنچے گا؟",
        "荷物はいつ届きますか",
    ] {
        let multilingual = run(&app, &user, query, a.id(), None).await.unwrap();
        assert!(
            !multilingual.pending,
            "multilingual retrieval must be fully indexed"
        );
        assert!(
            matches!(
                multilingual.rows[0].id.as_str(),
                "hybrid-000" | "hybrid-001"
            ),
            "delivery passages must outrank unrelated garden notes"
        );
        assert!(multilingual
            .search
            .unwrap()
            .matches
            .iter()
            .any(|hit| hit.message_id == "hybrid-001" && hit.excerpt.is_some()));
    }
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[ignore = "requires isolated application/search PostgreSQL; DAKIA_TEST_HYBRID_POSTGRES_URL"]
async fn four_backend_hybrid_provider_and_authorization_contract() {
    let url = std::env::var("DAKIA_TEST_HYBRID_POSTGRES_URL").unwrap();
    for (application, search) in [
        (None, Some(url.as_str())),
        (Some(url.as_str()), None),
        (Some(url.as_str()), Some(url.as_str())),
    ] {
        let (_dir, app, user, a, provider) = fixture_backend(3, false, application, search).await;
        let complete = run(&app, &user, "package", a.id(), None).await.unwrap();
        assert_eq!(complete.rows.len(), 3);
        assert!(!complete.pending);
        assert!(complete.search.unwrap().degradation.is_empty());
        assert!(complete.supplement.is_none());
        assert_eq!(provider.base.scans.load(Ordering::SeqCst), 0);
        stalled_bookkeeping_contract(&app, &user, &a, &provider).await;
        for (query, count) in [
            (r#""Package""#, 3),
            (r#"from:sender@example.invalid "Package""#, 3),
            (r#"from:absent@example.invalid "Package""#, 0),
            (r#""Package" "absent phrase""#, 0),
        ] {
            let strict = run(&app, &user, query, a.id(), None).await.unwrap();
            assert!(
                !strict.pending,
                "strict phrase validation must finish locally: {query}"
            );
            assert_eq!(strict.rows.len(), count, "{query}");
            assert!(strict.search.unwrap().degradation.is_empty());
        }
        let spelling = run(&app, &user, "packgae", a.id(), None).await.unwrap();
        assert!(!spelling.pending);
        assert_eq!(spelling.rows.len(), 3);
        assert!(
            spelling
                .search
                .unwrap()
                .matches
                .iter()
                .all(|hit| hit.score > 0.03),
            "both lexical and semantic ranks must contribute to typo results"
        );
        for query in [r#""packgae""#, "subject:packgae"] {
            let strict = run(&app, &user, query, a.id(), None).await.unwrap();
            assert!(
                strict.rows.is_empty() && !strict.pending,
                "phrases and structured filters must not expand"
            );
        }
        for (query, expected) in [
            ("package since:2020-01-01 before:2020-01-02", 3),
            ("package before:2020-01-01", 0),
            ("package since:2020-01-02", 0),
            ("since:2020-01-01 before:2020-01-02", 3),
        ] {
            let dated = run(&app, &user, query, a.id(), None).await.unwrap();
            assert!(!dated.pending, "caught-up date filtering must stay local");
            assert_eq!(dated.rows.len(), expected);
            assert!(dated.supplement.is_none());
        }
        for (since, before, count) in [
            ("2020-01-01", "2020-01-02", 3),
            ("2020-01-02", "2020-01-03", 0),
        ] {
            let cached = app
                .store
                .cached_search_rows(
                    &a,
                    &dakia_storage::CachedSearchFilter {
                        since: Some(since),
                        before: Some(before),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                cached.len(),
                count,
                "metadata previews must preserve date constraints"
            );
        }
        if let Some(search) = search {
            let pool = sqlx::PgPool::connect(search).await.unwrap();
            for table in [
                "dakia_search.chunks",
                "dakia_search.lexical_chunks",
                "dakia_search.spelling_signatures",
            ] {
                let mut lock = pool.begin().await.unwrap();
                sqlx::query(&format!("LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"))
                    .execute(&mut *lock)
                    .await
                    .unwrap();
                let started = std::time::Instant::now();
                let stalled = run(&app, &user, "package", a.id(), None).await.unwrap();
                assert!(started.elapsed() < Duration::from_secs(16));
                assert_eq!(
                    stalled.rows.len(),
                    3,
                    "{table} timeout erased healthy positives"
                );
                assert!(stalled.pending);
                assert!(!stalled.search.unwrap().degradation.is_empty());
                assert!(stalled.supplement.is_some());
                lock.rollback().await.unwrap();
            }
            pool.close().await;
        }
        let phrase = run(&app, &user, r#""package delivery""#, a.id(), None)
            .await
            .unwrap();
        assert!(phrase.rows.is_empty() && !phrase.pending);
        app.store.reset_scan(&a, "INBOX").await.unwrap();
        let partial = run(&app, &user, "package", a.id(), None).await.unwrap();
        assert!(partial.pending);
        assert_eq!(partial.rows.len(), 3);
        let token = partial.supplement.unwrap().remove(0).continuation;
        let guest = app
            .store
            .user("fixture", &uuid::Uuid::new_v4().to_string())
            .await
            .unwrap();
        assert!(run(&app, &guest, "package", a.id(), Some(token.clone()))
            .await
            .is_err());
        assert!(run(&app, &user, "different", a.id(), Some(token.clone()))
            .await
            .is_err());
        let supplemented = run(&app, &user, "package", a.id(), Some(token))
            .await
            .unwrap();
        assert!(
            !supplemented.rows.is_empty(),
            "application_pg={} search_pg={} coverage={:?}",
            application.is_some(),
            search.is_some(),
            supplemented.coverage
        );
        assert!(provider.base.scans.load(Ordering::SeqCst) > 0);
        assert!(supplemented
            .search
            .unwrap()
            .degradation
            .iter()
            .any(|d| d.code == "provider_literal_only"));
        if let Some(application) = application {
            let options: sqlx::postgres::PgConnectOptions = application.parse().unwrap();
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options.options([("search_path", "dakia_mail,public")]))
                .await
                .unwrap();
            let mut legacy = app.store.message(&a, "hybrid-000").await.unwrap();
            legacy.received_date = None;
            app.store.upsert_message(&a, legacy).await.unwrap();
            sqlx::query("UPDATE folder_scans SET completed_at=123,last_scanned_at=123,initial_boundary=42,continuation=NULL WHERE account_id=$1")
                .bind(a.id()).execute(&pool).await.unwrap();
            let generation: i64 = sqlx::query_scalar(
                "SELECT generation FROM folder_scans WHERE account_id=$1 AND folder='INBOX'",
            )
            .bind(a.id())
            .fetch_one(&pool)
            .await
            .unwrap();
            sqlx::raw_sql(include_str!(
                "../../storage/migrations/account_postgres/0019_internal_date.sql"
            ))
            .execute(&pool)
            .await
            .unwrap();
            let state: (i64, Option<i64>, Option<i64>) = sqlx::query_as("SELECT generation,completed_at,initial_boundary FROM folder_scans WHERE account_id=$1 AND folder='INBOX'")
                .bind(a.id()).fetch_one(&pool).await.unwrap();
            assert_eq!(state, (generation + 1, None, Some(42)));
            assert_eq!(
                app.store
                    .message(&a, "hybrid-000")
                    .await
                    .unwrap()
                    .received_date,
                None
            );
            pool.close().await;
        }
        let input = app.store.credential(&a).await.unwrap();
        app.store.remove_account(&a, &input.email).await.unwrap();
    }
}

#[tokio::test]
async fn broken_account_source_does_not_hide_healthy_hybrid_results() {
    let (dir, app, user, a, _provider) = fixture_backend(3, false, None, None).await;
    let input = app.store.credential(&a).await.unwrap();
    let account = app.store.connect(&user, &input).await.unwrap();
    let bad = app.store.authorize(&user, &account.id).await.unwrap();
    app.store.index_search_headers(&bad).await.unwrap();
    let db = database(&dir, &bad).await;
    sqlx::query("DROP TABLE provider_documents")
        .execute(&db)
        .await
        .unwrap();
    let page = run(
        &app,
        &user,
        "package",
        &format!("{},{}", a.id(), bad.id()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(page.rows.len(), 3);
    assert!(page.rows.iter().all(|m| m.account_id == a.id()));
    assert_eq!(
        page.coverage
            .iter()
            .find(|c| c.account_id == a.id())
            .unwrap()
            .state,
        CoverageState::Complete
    );
    assert_eq!(
        page.coverage
            .iter()
            .find(|c| c.account_id == bad.id())
            .unwrap()
            .state,
        CoverageState::Error
    );
    let lanes = page.supplement.unwrap();
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].account_id, bad.id());
}

#[tokio::test]
async fn progress_bookkeeping_failure_preserves_both_retrievers() {
    if let Ok(filter) = std::env::var("DAKIA_TEST_SEARCH_TRACE") {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    }
    let (dir, app, user, a, _provider) = fixture_backend(3, false, None, None).await;
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        dir.path().join("hybrid-search.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("DROP TABLE account_progress")
        .execute(&db)
        .await
        .unwrap();
    let page = run(&app, &user, "package", a.id(), None).await.unwrap();
    assert_eq!(page.rows.len(), 3, "{page:?}");
    assert!(page.pending);
    let details = page.search.unwrap();
    assert!(details.ranked);
    assert!(
        details.matches.iter().any(|m| m.excerpt.is_some()),
        "healthy vector retrieval survives bookkeeping failure: {details:?}"
    );
    assert!(details
        .degradation
        .iter()
        .any(|d| d.code == "search_coverage_unavailable"));
    assert_eq!(page.supplement.unwrap().len(), 1);
}
#[tokio::test]
async fn unresponsive_search_connection_has_a_bounded_provider_handoff() {
    let (_dir, mut app, user, a, provider) = fixture_backend(3, false, None, None).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stalled = tokio::spawn(async move {
        let mut sockets = vec![];
        while let Ok((socket, _)) = listener.accept().await {
            sockets.push(socket);
        }
    });
    let input = app.store.credential(&a).await.unwrap();
    let mut accounts = vec![a.id().to_owned()];
    for _ in 0..4 {
        let connected = app.store.connect(&user, &input).await.unwrap();
        accounts.push(connected.id);
    }
    app.store
        .configure_search_database(Some(&format!(
            "postgres://fixture:fixture@{address}/dakia_stalled_test"
        )))
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let page = run(&app, &user, "package", &accounts.join(","), None)
        .await
        .unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(16),
        "a stalled handshake must not delay five provider lanes for seconds"
    );
    assert!(page.pending);
    assert_eq!(page.supplement.unwrap().len(), 5);
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), 0);
    stalled.abort();
}

#[tokio::test]
async fn stalled_search_bookkeeping_retains_retrievers_and_recovers() {
    let (_dir, app, user, a, provider) = fixture_backend(3, false, None, None).await;
    stalled_bookkeeping_contract(&app, &user, &a, &provider).await;
}

async fn stalled_bookkeeping_contract(
    app: &crate::App,
    user: &str,
    a: &AuthorizedAccount,
    provider: &Controlled,
) {
    let held = app.store.hold_search_control_for_test().await.unwrap();
    let started = std::time::Instant::now();
    let page = run(app, user, "package", a.id(), None).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(16));
    assert_eq!(page.rows.len(), 3, "{page:?}");
    assert!(page.pending);
    let details = page.search.unwrap();
    assert_eq!(details.matches.len(), 3);
    // A contribution from either list alone cannot exceed 1/(60+1).
    assert!(
        details.matches.iter().all(|hit| hit.score > 1. / 61.),
        "both retrievers must contribute: {details:?}"
    );
    assert!(
        details.matches.iter().any(|hit| hit.excerpt.is_some()),
        "{details:?}"
    );
    assert!(
        details
            .degradation
            .iter()
            .any(|d| d.code == "search_coverage_unavailable"),
        "{details:?}"
    );
    assert_eq!(page.supplement.unwrap().len(), 1);
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), 0);
    drop(held);
    let recovered = run(app, user, "package", a.id(), None).await.unwrap();
    assert_eq!(recovered.rows.len(), 3);
    assert!(!recovered.pending, "{recovered:?}");
    assert!(recovered.search.unwrap().degradation.is_empty());
    assert!(recovered.supplement.is_none());
}

#[tokio::test]
async fn indexed_calendar_dates_are_strict_and_unknown_days_keep_provider_fallback() {
    let (_dir, app, user, a, provider) = fixture_backend(3, false, None, None).await;
    let mut message = app.store.message(&a, "hybrid-000").await.unwrap();
    // The UTC instant is Jan 1, but a provider west of UTC reports Dec 31.
    message.received_date = Some("2019-12-31".into());
    app.store.upsert_message(&a, message).await.unwrap();
    app.store.index_search_headers(&a).await.unwrap();
    app.store.flush_search_index(&a).await.unwrap();
    app.store.flush_embedding_receipts(&a).await.unwrap();
    let day = run(
        &app,
        &user,
        "package since:2019-12-31 before:2020-01-01",
        a.id(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        day.rows.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["hybrid-000"]
    );
    assert!(!day.pending);
    let next = run(
        &app,
        &user,
        "package since:2020-01-01 before:2020-01-02",
        a.id(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(next.rows.len(), 2);
    assert!(!next.pending);
    let mut unknown = app.store.message(&a, "hybrid-001").await.unwrap();
    unknown.received_date = None;
    app.store.upsert_message(&a, unknown).await.unwrap();
    app.store.index_search_headers(&a).await.unwrap();
    app.store.flush_search_index(&a).await.unwrap();
    app.store.flush_embedding_receipts(&a).await.unwrap();
    let partial = run(&app, &user, "package since:2020-01-01", a.id(), None)
        .await
        .unwrap();
    assert_eq!(
        partial
            .rows
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["hybrid-002"]
    );
    assert!(partial.pending);
    assert!(partial.supplement.is_some());
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[ignore = "requires pinned public E5 assets; local natural-language relevance"]
async fn real_hybrid_lisbon_question_finds_accommodation_without_literal_words() {
    let (_dir, app, user, a, provider) = fixture_backend(9, true, None, None).await;
    let before = provider.base.scans.load(Ordering::SeqCst);
    let page = run(&app, &user, "Where is my hotel in Lisbon?", a.id(), None)
        .await
        .unwrap();
    assert!(!page.pending);
    assert!(page.search.as_ref().unwrap().degradation.is_empty());
    let first: BTreeSet<_> = page.rows.iter().take(3).map(|m| m.id.as_str()).collect();
    assert_eq!(
        first,
        BTreeSet::from(["hybrid-003", "hybrid-004", "hybrid-005"]),
        "accommodation should outrank restaurants, other cities and unrelated mail: {:?}",
        page.rows
            .iter()
            .map(|m| (&m.id, &m.subject))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        provider.base.scans.load(Ordering::SeqCst),
        before,
        "a complete local index must answer without provider search"
    );
}
