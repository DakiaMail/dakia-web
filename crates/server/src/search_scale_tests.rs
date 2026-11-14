//! Opt-in CPU release profiling. Synthetic corpus, real storage/model/router/TCP.
//! HTTP timing is an intermediate measurement, never submission-to-render proof.
use super::*;
use base64::Engine;
use dakia_inference::embedding::{search_model_revision, LocalSearchEmbedder};
use dakia_storage::{EmbeddedPassage, SearchContent, Store};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, io::Read};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "release scale workload: 250k messages, five accounts, pinned E5; retains fixture/report"]
async fn release_mailbox_search_scale() {
    assert!(!cfg!(debug_assertions), "run with cargo test --release");
    let count: usize = std::env::var("DAKIA_BENCH_MESSAGES")
        .unwrap_or_else(|_| "250000".into())
        .parse()
        .unwrap();
    let samples: usize = std::env::var("DAKIA_BENCH_QUERIES")
        .unwrap_or_else(|_| "1000".into())
        .parse()
        .unwrap();
    assert!(count >= 5 && count % 5 == 0 && samples > 0);
    let application = std::env::var("DAKIA_BENCH_DATABASE_URL")
        .ok()
        .filter(|v| !v.is_empty());
    let mut search = std::env::var("DAKIA_BENCH_SEARCH_DATABASE_URL")
        .ok()
        .filter(|v| !v.is_empty());
    for url in [application.as_deref(), search.as_deref()]
        .into_iter()
        .flatten()
        .filter(|url| *url != "sqlite")
    {
        assert!(
            url.starts_with("postgres"),
            "benchmark URL overrides must use isolated PostgreSQL"
        );
        assert_eq!(
            url::Url::parse(url).unwrap().path(),
            "/dakia_search_bench",
            "only the isolated benchmark database is allowed"
        );
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(url)
            .await
            .unwrap();
        for table in ["dakia_control.accounts", "dakia_search.documents"] {
            let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
                .bind(table)
                .fetch_one(&db)
                .await
                .unwrap();
            if exists {
                let rows: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                    .fetch_one(&db)
                    .await
                    .unwrap();
                assert_eq!(
                    rows, 0,
                    "benchmark requires empty isolated storage: {table}"
                );
            }
        }
        db.close().await;
    }
    let (dir, mut app, _, original, _) = controlled().await;
    let input = app.store.credential(&original).await.unwrap();
    let root = dir.keep();
    println!("SCALE_ROOT={}", root.display());
    if search.as_deref() == Some("sqlite") {
        search = Some(format!(
            "sqlite:{}",
            root.join("scale-search.sqlite").display()
        ));
    }
    let search_postgres = search
        .as_deref()
        .map(|url| url.starts_with("postgres"))
        .unwrap_or(application.is_some());
    app.store = Store::open_with_database(
        root.join("scale-app"),
        [27; 32],
        1024 * 1024 * 1024,
        application.as_deref(),
    )
    .await
    .unwrap();
    app.store
        .control_repository()
        .put_setting("jobs_enabled", "false")
        .await
        .unwrap();
    std::fs::write(root.join("credential-key"), [27; 32]).unwrap();
    let user = app
        .store
        .user("fixture", &uuid::Uuid::new_v4().to_string())
        .await
        .unwrap();
    app.store
        .configure_search_database(search.as_deref())
        .await
        .unwrap();
    let revision = search_model_revision();
    app.store.require_search_embeddings(&revision).unwrap();
    app.store.maintain_search_vectors(false).await.unwrap();
    let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../inference/assets/multilingual-e5-small");
    let cold = std::time::Instant::now();
    let mut model = LocalSearchEmbedder::from_dir(&model_path).unwrap();
    let model_load_ms = cold.elapsed().as_secs_f64() * 1000.;
    app.mail = app.mail.with_search_model(model_path);
    let topics = [
        "package delivery courier shipment",
        "garden roses sunlight watering",
        "invoice payment bank transfer",
        "meeting schedule project review",
        "travel flight hotel reservation",
        "doctor appointment medical report",
        "software release code review",
        "family dinner birthday invitation",
    ];
    let mut scopes = vec![];
    let mut text_bytes = 0usize;
    for index in 0..5 {
        let mut input = input.clone();
        input.email = format!("scale-{index}@example.invalid");
        let account = app.store.connect(&user, &input).await.unwrap();
        let a = app.store.authorize(&user, &account.id).await.unwrap();
        let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
        for start in (0..count / 5).step_by(200) {
            let end = (start + 200).min(count / 5);
            let mut messages = vec![];
            for n in start..end {
                let id = format!("scale-{index}-{n:07}");
                let m: Message = serde_json::from_value(serde_json::json!({"id":id,"account_id":a.id(),"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":format!("person{}@example.invalid",n%64),"name":format!("Person {}",n%64)},"to":[],"subject":topics[n%8],"preview":"","received_at":format!("2026-10-{:02}T00:00:00Z",n%28+1),"received_date":format!("2026-10-{:02}",n%28+1),"unread":n%2==0,"starred":n%7==0,"category":null,"category_status":"pending","revision":1})).unwrap();
                messages.push(m);
            }
            let messages = app.store.ingest_messages(&a, messages, &[]).await.unwrap();
            app.store.index_search_headers(&a).await.unwrap();
            let bodies: Vec<_> = (start..end)
                .map(|n| {
                    format!(
                        "reference{:04} {} {}",
                        n % 1000,
                        topics[n % 8],
                        "Historical correspondence. café 東京 Tallinn arve makse. ".repeat(44)
                    )
                })
                .collect();
            let attachments: Vec<_> = (start..end)
                .map(|n| {
                    format!(
                        "report-{}.txt\nattachmentreference{:04} {}",
                        n % 8,
                        n % 1000,
                        "Local extracted attachment document details. ".repeat(24)
                    )
                })
                .collect();
            let records: Vec<_> = messages
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    text_bytes += bodies[i].len() + attachments[i].len();
                    SearchContent {
                        message: m,
                        text: &bodies[i],
                        state: "ready",
                        participants: "Reply Person reply@example.invalid",
                        recipients: "carbon@example.invalid",
                        attachments: &attachments[i],
                        attachment_details: "[]",
                    }
                })
                .collect();
            app.store.put_search_records(&a, &records).await.unwrap();
            app.store
                .record_scan(
                    &a,
                    "INBOX",
                    generation,
                    &scan_page(&a, messages, end < count / 5),
                )
                .await
                .unwrap();
            while app.store.flush_search_index(&a).await.unwrap() {}
            if start % 10000 == 0 {
                println!("SCALE_DISCOVERY account={index} messages={end}");
            }
        }
        app.store
            .prepare_embeddings(&a, &revision, true)
            .await
            .unwrap();
        while app.store.discover_embeddings(&a, &revision).await.unwrap() {}
        scopes.push(a);
    }
    let token = "synthetic-scale-owner";
    app.store
        .control_repository()
        .create_session(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(token.as_bytes())),
            &user,
            "scale-csrf",
            4102444800,
        )
        .await
        .unwrap();
    let metadata = serde_json::json!({"messages":count,"accounts":5,"text_bytes":text_bytes,"model_load_ms":model_load_ms,"application_postgres":application.is_some(),"search_postgres":search_postgres});
    std::fs::write(
        root.join("scale-fixture.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    println!("SCALE_FIXTURE_READY={}", root.display());
    finish_embeddings(&app, &scopes, &mut model, &revision).await;
    while app.store.maintain_search_vectors(true).await.unwrap() {}
    for a in &scopes {
        let c = app.store.text_coverage(a).await.unwrap();
        assert_eq!(c.processed, count as i64 / 5, "incomplete fixture: {c:?}");
    }
    refresh(&app, &scopes).await;
    measure(app, &root, &scopes, metadata, samples, "scale-report.json").await;
}

async fn finish_embeddings(
    app: &crate::App,
    scopes: &[AuthorizedAccount],
    model: &mut LocalSearchEmbedder,
    revision: &str,
) {
    // Reusing an identical passage's individually inferred vector changes no
    // model output. This cache is fixture construction only, outside measurement.
    let mut cache: HashMap<String, Vec<f32>> = HashMap::new();
    let mut turns = 0usize;
    for a in scopes {
        while let Some(turn) = app.store.claim_embedding(a, revision, true).await.unwrap() {
            let passage = if turn.text.trim().is_empty() {
                EmbeddedPassage {
                    text: turn.text.clone(),
                    byte_end: turn.text.len(),
                    next_offset: turn.text.len(),
                    vector: None,
                }
            } else {
                let passage = model.next_passage(&turn.text, 0).unwrap().unwrap();
                let vector = if let Some(v) = cache.get(&passage.text) {
                    v.clone()
                } else {
                    let v = model
                        .embed_passages(&[&passage.text])
                        .unwrap()
                        .remove(0)
                        .to_vec();
                    if cache.len() < 8192 {
                        cache.insert(passage.text.clone(), v.clone());
                    }
                    v
                };
                EmbeddedPassage {
                    text: passage.text,
                    byte_end: passage.end,
                    next_offset: passage.next_offset,
                    vector: Some(vector),
                }
            };
            assert!(app
                .store
                .commit_embedding(a, &turn, &passage)
                .await
                .unwrap());
            turns += 1;
            if turns % 10000 == 0 {
                println!(
                    "SCALE_EMBEDDING turns={turns} cached_passages={}",
                    cache.len()
                );
            }
        }
        while app.store.advance_search_lexical(a, true).await.unwrap() {}
        while app.store.flush_embedding_receipts(a).await.unwrap() {}
    }
    // A retained fixture is usable only after all leases and queued work have drained.
    for a in scopes {
        let coverage = app.store.embedding_coverage(a).await.unwrap();
        assert_eq!(
            coverage.pending, 0,
            "fixture still has unfinished embeddings: {coverage:?}"
        );
    }
}

fn scale_query(n: usize, workload: &str) -> String {
    let reference = format!("reference{:04}", n % 1000);
    match if workload == "references" {
        n % 6
    } else {
        n % 24
    } {
        0 => reference,
        1 => format!("attachmentreference{:04}", n % 1000),
        2 => format!("from:person{}@example.invalid {reference}", n % 64),
        3 => format!("\"historical correspondence\" {reference}"),
        4 => format!("pakcage delivrey {reference}"),
        5 => format!("arve makse 東京 {reference}"),
        6 => "\"package delivery\"".into(),
        7 => "\"Historical correspondence\"".into(),
        8 => "garden".into(),
        9 => format!("from:person{}@example.invalid", n % 64),
        10 => "subject:\"meeting schedule\"".into(),
        11 => "to:carbon@example.invalid".into(),
        12 => "invoice payment".into(),
        13 => "where is my parcel".into(),
        14 => "kus on minu pakk".into(),
        15 => "荷物の配達".into(),
        16 => "доставка посылки".into(),
        17 => "pakcage delivrey".into(),
        18 => "is:unread invoice".into(),
        19 => "is:starred package since:2026-10-01 before:2026-10-15".into(),
        20 => format!(
            "from:person{}@example.invalid \"Historical correspondence\"",
            n % 64
        ),
        21 => format!("subject:absent-subject-{n:04}"),
        22 => format!("subject:absent-subject-{n:04} package"),
        _ => "\"Local extracted attachment\"".into(),
    }
}
#[test]
fn scale_workload_contains_broad_queries_and_strict_filter_only_searches() {
    let plans: Vec<_> = (0..24)
        .map(|n| {
            dakia_domain::mailbox_query::MailboxQuery::parse(&scale_query(n, "mixed")).unwrap()
        })
        .collect();
    assert_eq!(
        plans
            .iter()
            .filter(|plan| plan.text_units().is_empty())
            .count(),
        4
    );
    assert_eq!(plans[7].phrases(), vec!["Historical correspondence"]);
    assert!(scale_query(8, "mixed").contains("garden"));
    assert!((0..24).all(|n| !scale_query(n, "mixed").is_empty()));
}

async fn measure(
    app: crate::App,
    root: &std::path::Path,
    scopes: &[AuthorizedAccount],
    mut metadata: serde_json::Value,
    samples: usize,
    filename: &str,
) {
    let token = "synthetic-scale-owner";
    let mut binary = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = binary.read(&mut buffer).unwrap();
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    metadata["release_binary_sha256"] = serde_json::json!(format!("{:x}", digest.finalize()));
    metadata["search_account_concurrency"] =
        serde_json::json!(dakia_storage::SEARCH_ACCOUNT_CONCURRENCY);
    metadata["read_connections_per_lane"] =
        serde_json::json!(dakia_storage::SEARCH_ACCOUNT_CONCURRENCY);
    let mut app = app;
    if let Ok(web_root) = std::env::var("DAKIA_BENCH_WEB_ROOT") {
        assert!(
            std::path::Path::new(&web_root).join("index.html").is_file(),
            "production web build is required"
        );
        std::sync::Arc::make_mut(&mut app.config).web_root = web_root;
    }
    // Repeated natural-language queries still execute the real model. Do not
    // satisfy the hybrid latency gate with cached query vectors.
    app.mail.search_queries = app.mail.search_queries.clone().uncached();
    let workload = std::env::var("DAKIA_BENCH_WORKLOAD").unwrap_or_else(|_| "mixed".into());
    assert!(matches!(workload.as_str(), "mixed" | "references"));
    metadata["workload"] = serde_json::json!(format!("{workload}-v1"));
    metadata["query_embedding_cache"] = serde_json::json!(false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    println!("SCALE_HTTP_BASE=http://{address}");
    let server =
        tokio::spawn(async move { axum::serve(listener, crate::router(app)).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let accounts = scopes.iter().map(|a| a.id()).collect::<Vec<_>>().join(",");
    let mut timings = vec![];
    let mut observations = vec![];
    let mut degraded = 0;
    for n in 0..samples {
        let query = scale_query(n, &workload);
        let expects_hybrid = !dakia_domain::mailbox_query::MailboxQuery::parse(&query)
            .unwrap()
            .text_units()
            .is_empty();
        let start = std::time::Instant::now();
        let response = client
            .get(format!("http://{address}/api/search"))
            .header("cookie", format!("__Host-dakia={token}"))
            .query(&[
                ("q", query.as_str()),
                ("accounts", accounts.as_str()),
                ("cache_first", "true"),
            ])
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "HTTP {}", response.status());
        let page: SearchPage = response.json().await.unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if page.pending
            || page
                .search
                .as_ref()
                .is_some_and(|s| !s.degradation.is_empty())
            || expects_hybrid && page.search.is_none()
            || scopes.iter().any(|account| {
                !page.coverage.iter().any(|coverage| {
                    coverage.account_id == account.id()
                        && coverage.state == CoverageState::Complete
                        && coverage.error.is_none()
                })
            })
        {
            degraded += 1;
        }
        if std::env::var_os("DAKIA_BENCH_TRACE").is_some() {
            println!(
                "SCALE_QUERY number={n} elapsed_ms={elapsed:.3} rows={} pending={}",
                page.rows.len(),
                page.pending
            );
        }
        observations.push(serde_json::json!({"query":query,"expects_hybrid":expects_hybrid,"elapsed_ms":elapsed,"rows":page.rows.len(),"pending":page.pending,"coverage":page.coverage,"degradation":page.search.as_ref().map(|s|&s.degradation)}));
        timings.push(elapsed);
    }
    let first_http_ms = timings[0];
    timings.sort_by(f64::total_cmp);
    let p95 = timings[((samples as f64 * 0.95).ceil() as usize).saturating_sub(1)];
    metadata["run_status"] = serde_json::json!("completed");
    metadata["scope"] = serde_json::json!("release HTTP response decoded, excludes rendering");
    metadata["queries"] = serde_json::json!(samples);
    metadata["first_http_ms"] = serde_json::json!(first_http_ms);
    metadata["p95_ms"] = serde_json::json!(p95);
    metadata["degraded"] = serde_json::json!(degraded);
    metadata["full_acceptance"] = serde_json::json!(false);
    metadata["samples"] = serde_json::json!(observations);
    let report = metadata;
    std::fs::write(
        root.join(filename),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("SCALE_REPORT_PATH={}", root.join(filename).display());
    println!("SCALE_RESULT p95_ms={p95:.3} degraded={degraded} samples={samples}");
    let hold: u64 = std::env::var("DAKIA_BENCH_HOLD_SECONDS")
        .ok()
        .map(|value| value.parse().unwrap())
        .unwrap_or(0);
    if hold > 0 {
        tokio::time::sleep(std::time::Duration::from_secs(hold)).await;
    }
    server.abort();
    assert_eq!(
        degraded, 0,
        "degraded previews cannot satisfy hybrid acceptance"
    );
    // Latency is reported for review; complete results remain mandatory.
}
fn scan_page(a: &AuthorizedAccount, rows: Vec<Message>, more: bool) -> SearchPage {
    serde_json::from_value(serde_json::json!({"rows":rows,"continuation":more.then_some("scale-next"),"coverage":[{"account_id":a.id(),"folder_ids":["INBOX"],"state":"complete","error":null}],"classification":{"classified":0,"total":0},"pending":more,"counts_exact":false})).unwrap()
}
async fn refresh(app: &crate::App, scopes: &[AuthorizedAccount]) {
    for a in scopes {
        app.store.put_provider_document(a,"sync:folders",&serde_json::json!({"folders":[{"name":"INBOX","selectable":true,"attributes":[]}],"index":0,"refreshed":chrono::Utc::now().timestamp()}).to_string()).await.unwrap();
        if let Some(generation) = app.store.completed_scan_due(a, "INBOX").await.unwrap() {
            app.store
                .confirm_unchanged_scan(a, "INBOX", generation)
                .await
                .unwrap();
        }
    }
}

async fn retained_fixture(
    upgrade: bool,
) -> (
    std::path::PathBuf,
    crate::App,
    Vec<AuthorizedAccount>,
    serde_json::Value,
) {
    assert!(!cfg!(debug_assertions), "run with cargo test --release");
    let root = std::path::PathBuf::from(std::env::var("DAKIA_BENCH_REUSE_ROOT").unwrap());
    if std::env::var_os("DAKIA_BENCH_TRACE").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("dakia_server::provider_io=info,dakia_server::diagnostics=warn,dakia_server::search_timing=debug")
            .try_init();
    }
    assert_eq!(
        std::fs::read(root.join("credential-key")).unwrap(),
        [27; 32]
    );
    let report_path = if root.join("scale-report.json").is_file() {
        root.join("scale-report.json")
    } else if root.join("scale-interrupted-diagnostic.json").is_file() {
        root.join("scale-interrupted-diagnostic.json")
    } else {
        root.join("scale-fixture.json")
    };
    let mut report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(report_path).expect("retained corpus report required"),
    )
    .unwrap();
    let application = std::env::var("DAKIA_BENCH_DATABASE_URL")
        .ok()
        .filter(|v| !v.is_empty());
    let mut search = std::env::var("DAKIA_BENCH_SEARCH_DATABASE_URL")
        .ok()
        .filter(|v| !v.is_empty());
    // Check overrides before opening either repository, which may migrate it.
    // Replay has the same isolated-database boundary as corpus construction.
    for connection in [application.as_deref(), search.as_deref()]
        .into_iter()
        .flatten()
        .filter(|connection| *connection != "sqlite")
    {
        assert!(connection.starts_with("postgres"));
        assert_eq!(
            url::Url::parse(connection).unwrap().path(),
            "/dakia_search_bench",
            "only the isolated benchmark database is allowed"
        );
    }
    let search_postgres = search
        .as_deref()
        .map(|v| v.starts_with("postgres"))
        .unwrap_or(application.is_some());
    assert_eq!(
        report["application_postgres"],
        serde_json::json!(application.is_some())
    );
    assert_eq!(
        report["search_postgres"],
        serde_json::json!(search_postgres)
    );
    if search.as_deref() == Some("sqlite") {
        search = Some(format!(
            "sqlite:{}",
            root.join("scale-search.sqlite").display()
        ));
    }
    let (_temporary, mut app, _, _, _) = controlled().await;
    let startup = std::time::Instant::now();
    app.store = Store::open_with_database(
        root.join("scale-app"),
        [27; 32],
        1024 * 1024 * 1024,
        application.as_deref(),
    )
    .await
    .unwrap();
    let session = app
        .store
        .control_repository()
        .session(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(b"synthetic-scale-owner")),
            chrono::Utc::now().timestamp(),
        )
        .await
        .unwrap()
        .expect("retained fixture session");
    let accounts = app.store.accounts(&session.user_id).await.unwrap();
    assert_eq!(accounts.len(), 5);
    assert!(
        accounts
            .iter()
            .all(|a| a.email.starts_with("scale-") && a.email.ends_with("@example.invalid")),
        "only synthetic scale fixtures may be replayed"
    );
    let mut scopes = vec![];
    for a in accounts {
        scopes.push(app.store.authorize(&session.user_id, &a.id).await.unwrap());
    }
    let revision = search_model_revision();
    app.store.require_search_embeddings(&revision).unwrap();
    app.store
        .configure_search_database(search.as_deref())
        .await
        .unwrap();
    let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../inference/assets/multilingual-e5-small");
    app.mail = app.mail.with_search_model(model_path.clone());
    if std::env::var_os("DAKIA_BENCH_FINISH_INDEXING").is_some() {
        let resumed = std::time::Instant::now();
        let mut model = LocalSearchEmbedder::from_dir(&model_path).unwrap();
        finish_embeddings(&app, &scopes, &mut model, &revision).await;
        report["fixture_resume_ms"] = serde_json::json!(resumed.elapsed().as_secs_f64() * 1000.);
        report["startup_includes_fixture_resume"] = serde_json::json!(true);
    }
    while app.store.maintain_search_vectors(true).await.unwrap() {}
    if upgrade {
        let upgrade_start = std::time::Instant::now();
        for a in &scopes {
            app.store
                .prepare_embeddings(a, &revision, true)
                .await
                .unwrap();
        }
        let mut turns = 0u64;
        loop {
            let mut advanced = false;
            for a in &scopes {
                advanced |= app.store.advance_search_lexical(a, true).await.unwrap();
                if turns % 200 == 0 {
                    while app.store.flush_embedding_receipts(a).await.unwrap() {}
                }
            }
            if !advanced {
                break;
            }
            turns += 1;
            if turns % 1000 == 0 {
                println!(
                    "SCALE_LEXICAL_UPGRADE_TURNS={turns} SECONDS={:.3}",
                    upgrade_start.elapsed().as_secs_f64()
                );
            }
        }
        for a in &scopes {
            while app.store.flush_embedding_receipts(a).await.unwrap() {}
        }
        report["lexical_upgrade_seconds"] =
            serde_json::json!(upgrade_start.elapsed().as_secs_f64());
        report["lexical_upgrade_turns"] = serde_json::json!(turns);
        println!(
            "SCALE_LEXICAL_UPGRADE_COMPLETE={:.3}",
            upgrade_start.elapsed().as_secs_f64()
        );
    }
    for a in &scopes {
        let coverage_start = std::time::Instant::now();
        let coverage = app.store.text_coverage(a).await.unwrap();
        println!(
            "SCALE_COVERAGE_MS={:.3}",
            coverage_start.elapsed().as_secs_f64() * 1000.
        );
        assert_eq!(coverage.eligible, report["messages"].as_i64().unwrap() / 5);
        assert_eq!(
            coverage.processed, coverage.eligible,
            "replay requires completed content processing"
        );
    }
    while app.mail.search_queries.state() != crate::search_query_model::QueryModelState::Ready {
        assert!(
            startup.elapsed() < std::time::Duration::from_secs(300),
            "query model did not become ready"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    report["replay_startup_ms"] = serde_json::json!(startup.elapsed().as_secs_f64() * 1000.);
    for key in [
        "samples",
        "observations",
        "persisted_query_scopes",
        "degraded_scopes",
        "cursor_timestamp_span_seconds",
        "captured_at",
        "queries_requested",
    ] {
        report.as_object_mut().unwrap().remove(key);
    }
    refresh(&app, &scopes).await;
    (root, app, scopes, report)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires completed retained synthetic scale fixture; DAKIA_BENCH_REUSE_ROOT"]
async fn release_mailbox_search_replay() {
    let (root, app, scopes, report) = retained_fixture(false).await;
    let samples: usize = std::env::var("DAKIA_BENCH_QUERIES")
        .unwrap_or_else(|_| "1000".into())
        .parse()
        .unwrap();
    assert!(samples > 0);
    let filename = format!("scale-replay-{}.json", uuid::Uuid::new_v4());
    measure(app, &root, &scopes, report, samples, &filename).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "lexical-only retained synthetic fixture upgrade, never latency acceptance"]
async fn release_mailbox_search_upgrade() {
    let (root, _app, _scopes, mut report) = retained_fixture(true).await;
    report["run_status"] = serde_json::json!("lexical_upgrade_completed");
    report["scope"] = serde_json::json!(
        "retained lexical processing and receipt recovery, excludes search latency"
    );
    if let Some(elapsed) = report.as_object_mut().unwrap().remove("replay_startup_ms") {
        report["fixture_open_and_lexical_upgrade_ms"] = elapsed;
    }
    std::fs::write(
        root.join("scale-lexical-upgrade.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "component profiling only, never latency acceptance; DAKIA_BENCH_REUSE_ROOT"]
async fn release_mailbox_search_components() {
    let (root, app, scopes, mut report) = retained_fixture(false).await;
    let a = &scopes[0];
    let query =
        std::env::var("DAKIA_BENCH_COMPONENT_QUERY").unwrap_or_else(|_| "reference0000".into());
    let plan = dakia_domain::mailbox_query::MailboxQuery::parse(&query).unwrap();
    let units = plan.text_units();
    let at = std::time::Instant::now();
    let vector = app
        .mail
        .search_queries
        .embed(&plan.semantic_text())
        .await
        .unwrap();
    let embedding_ms = at.elapsed().as_secs_f64() * 1000.;
    println!("SCALE_COMPONENT embedding_ms={embedding_ms:.3}");
    let at = std::time::Instant::now();
    let alternatives = app.store.search_spelling(a, &units, &[true]).await.unwrap();
    let spelling_ms = at.elapsed().as_secs_f64() * 1000.;
    let filter = dakia_storage::CachedSearchFilter {
        text: Some(&query),
        text_units: Some(&units),
        text_alternatives: Some(&alternatives),
        ..Default::default()
    };
    let at = std::time::Instant::now();
    let lexical = app
        .store
        .full_text_ranked_candidates(a, &filter, None, 513)
        .await
        .unwrap();
    let lexical_ms = at.elapsed().as_secs_f64() * 1000.;
    println!(
        "SCALE_COMPONENT lexical_ms={lexical_ms:.3} rows={}",
        lexical.len()
    );
    let at = std::time::Instant::now();
    let semantic = app
        .store
        .semantic_search_rows(a, &filter, &search_model_revision(), &vector)
        .await
        .unwrap();
    let semantic_ms = at.elapsed().as_secs_f64() * 1000.;
    println!(
        "SCALE_COMPONENT semantic_ms={semantic_ms:.3} rows={}",
        semantic.len()
    );
    report["scope"] = serde_json::json!(
        "single account sequential component diagnostics, excludes HTTP/rendering"
    );
    report["full_acceptance"] = serde_json::json!(false);
    report["component_query"] = serde_json::json!(query);
    report["embedding_ms"] = serde_json::json!(embedding_ms);
    report["spelling_ms"] = serde_json::json!(spelling_ms);
    report["lexical_ms"] = serde_json::json!(lexical_ms);
    report["lexical_rows"] = serde_json::json!(lexical.len());
    report["semantic_ms"] = serde_json::json!(semantic_ms);
    report["semantic_rows"] = serde_json::json!(semantic.len());
    let filename = root.join(format!("scale-components-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&filename, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("SCALE_COMPONENT_REPORT={}", filename.display());
}
