//! Real TCP server smoke with synthetic local state, jobs disabled and no provider I/O.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use dakia_domain::*;
use dakia_storage::Store;
use sha2::{Digest, Sha256};

async fn runtime_contract(application: Option<&str>, search: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let mut store = Store::open_with_database(&data, [8; 32], 10, application)
        .await
        .unwrap();
    store.configure_search_database(search).await.unwrap();
    let user = store
        .user("https://id.example.invalid", "owner")
        .await
        .unwrap();
    for (subject, token) in [("owner", "owner-fixture"), ("other", "other-fixture")] {
        let user = store
            .user("https://id.example.invalid", subject)
            .await
            .unwrap();
        store
            .control_repository()
            .create_session(
                &URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes())),
                &user,
                "fixture-csrf",
                4102444800,
            )
            .await
            .unwrap();
    }
    store
        .control_repository()
        .put_setting("jobs_enabled", "false")
        .await
        .unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let scope = store.authorize(&user, &account.id).await.unwrap();
    let message: Message = serde_json::from_value(serde_json::json!({"id":"synthetic-message","account_id":account.id,"thread_id":"thread","provider_id":"stable","folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},"to":[],"subject":"Synthetic runtime test","preview":"","received_at":"2026-10-28T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    let message = store.upsert_message(&scope, message).await.unwrap();
    let content: MessageContent = serde_json::from_value(serde_json::json!({
        "message": message, "text": "Please review the attached meeting notes before our discussion tomorrow. We will confirm the final project schedule and answer any remaining questions together.",
        "html": "", "attachments": [], "message_id": null, "references": [], "cc": []
    })).unwrap();
    store
        .cache_content(&scope, &content, 1024 * 1024)
        .await
        .unwrap();
    store.index_search_headers(&scope).await.unwrap();
    store
        .put_search_body(
            &scope,
            &message,
            "runtimebody lexical token",
            "ready",
            Some(false),
        )
        .await
        .unwrap();
    // Wait for the selected destination before launching the separately compiled
    // server, so body matching exercises persisted search through real HTTP.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if store.flush_search_index(&scope).await.is_ok() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "selected search store did not become ready"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    drop(store);
    let key_path = dir.path().join("key");
    let oidc_path = dir.path().join("oidc");
    std::fs::write(&key_path, [8; 32]).unwrap();
    std::fs::write(&oidc_path, b"synthetic-only").unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let binary = std::env::var("DAKIA_TEST_SERVER_BINARY")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_dakia-server").into());
    let mut command = tokio::process::Command::new(binary);
    command
        .current_dir(&root)
        .kill_on_drop(true)
        .env("DAKIA_ORIGIN", "https://dakia.example.invalid")
        .env("DAKIA_OIDC_ISSUER", "https://id.example.invalid")
        .env("DAKIA_OIDC_CLIENT_ID", "fixture")
        .env("DAKIA_OIDC_SECRET_FILE", &oidc_path)
        .env("DAKIA_CREDENTIAL_KEY_FILE", &key_path)
        .env("DAKIA_DATA_DIR", &data)
        .env("DAKIA_LISTEN", address.to_string())
        .env("DAKIA_DURABLE_ATTACHMENT_BUDGET_BYTES", "10")
        .env("DAKIA_WORKERS", root.join("workers"))
        .env(
            "DAKIA_SEARCH_MODEL_DIR",
            dir.path().join("absent-search-model"),
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .env("DAKIA_DATABASE_URL", application.unwrap_or(""))
        .env("DAKIA_SEARCH_DATABASE_URL", search.unwrap_or(""));
    let mut child = command.spawn().unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("http://{address}");
    let startup_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if client
            .get(format!("{base}/healthz"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "server exited during startup"
        );
        assert!(
            tokio::time::Instant::now() < startup_deadline,
            "fixture server did not become healthy within 15 seconds"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        client
            .get(format!("{base}/healthz"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let request = |method, path: &str, token: &str| {
        client
            .request(method, format!("{base}/api{path}"))
            .header("cookie", format!("__Host-dakia={token}"))
            .header("origin", "https://dakia.example.invalid")
            .header("x-csrf-token", "fixture-csrf")
    };
    // The real binary must answer from application storage without resolving the fictional
    // IMAP host. No test-only provider or API mock exists in this process.
    let preview_path = format!(
        "/search?q=subject:Synthetic&accounts={}&cache_first=true",
        account.id
    );
    let preview: SearchPage = request(reqwest::Method::GET, &preview_path, "owner-fixture")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview.cache_preview, Some(true));
    assert_eq!(preview.rows.len(), 1);
    assert_eq!(preview.rows[0].id, message.id);
    assert!(preview.pending && !preview.counts_exact && preview.continuation.is_some());
    assert_eq!(
        request(reqwest::Method::GET, &preview_path, "other-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let indexed: SearchPage = request(
        reqwest::Method::GET,
        &format!(
            "/search?q=runtimebody&accounts={}&cache_first=true",
            account.id
        ),
        "owner-fixture",
    )
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(indexed.pending);
    assert_eq!(indexed.rows.len(), 1);
    assert_eq!(indexed.rows[0].id, message.id);
    let progress: serde_json::Value = request(
        reqwest::Method::GET,
        &format!("/accounts/{}/indexing", account.id),
        "owner-fixture",
    )
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    // The fixture deliberately has no model assets. Lexical delivery alone
    // must not claim that semantic processing has finished.
    assert_eq!(progress["processed"], 0);
    assert_eq!(progress["backlog"], 1);
    assert_eq!(progress["state"], "paused");
    assert_eq!(progress["eligible"], 1);
    let translation = format!(
        "/accounts/{}/messages/{}/translation",
        account.id, message.id
    );
    let sender_path = format!(
        "/accounts/{}/messages/{}/sender-mail",
        account.id, message.id
    );
    let sender_page: serde_json::Value =
        request(reqwest::Method::GET, &sender_path, "owner-fixture")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(sender_page["count"], 1);
    assert_eq!(sender_page["rows"][0]["id"], message.id);
    assert_eq!(sender_page["rows"][0]["unread"], true);
    assert_eq!(
        request(reqwest::Method::GET, &sender_path, "other-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let detected: MessageContent = request(
        reqwest::Method::GET,
        &format!("/accounts/{}/messages/{}", account.id, message.id),
        "owner-fixture",
    )
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(detected.detected_language.as_deref(), Some("en"));
    let found: DiscoveredSettings =
        request(reqwest::Method::POST, "/accounts/discover", "owner-fixture")
            .json(&serde_json::json!({"email": "synthetic@gmail.com"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(found.imap_host, "imap.gmail.com");
    assert_eq!(found.smtp_port, 465);
    assert_eq!(
        request(
            reqwest::Method::POST,
            &format!("{translation}?language=et"),
            "other-fixture"
        )
        .send()
        .await
        .unwrap()
        .status(),
        404
    );
    assert_eq!(
        request(
            reqwest::Method::POST,
            &format!("{translation}?language=xx"),
            "owner-fixture"
        )
        .send()
        .await
        .unwrap()
        .status(),
        400
    );
    let job: ModelJob = request(
        reqwest::Method::POST,
        &format!("{translation}?language=et"),
        "owner-fixture",
    )
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(job.account_id, account.id);
    assert_eq!(job.status, JobStatus::Pending);
    let job_path = format!("/accounts/{}/jobs/{}", account.id, job.id);
    assert_eq!(
        request(reqwest::Method::GET, &job_path, "other-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        request(reqwest::Method::GET, &job_path, "owner-fixture")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        request(
            reqwest::Method::GET,
            &format!("{translation}?language=et"),
            "owner-fixture"
        )
        .send()
        .await
        .unwrap()
        .status(),
        409
    );
    let operations = format!("/accounts/{}/operations", account.id);
    let operation = Operation {
        id: "runtime-star".into(),
        account_id: account.id.clone(),
        target_id: message.id.clone(),
        expected_revision: message.revision,
        change: Change::Star { value: true },
        status: OperationStatus::Pending,
        revision: 0,
    };
    let mut results = vec![];
    for _ in 0..2 {
        let response = request(reqwest::Method::POST, &operations, "owner-fixture")
            .json(&operation)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        results.push(response.json::<Operation>().await.unwrap());
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0].status, OperationStatus::Pending);
    assert_eq!(
        request(reqwest::Method::POST, &operations, "other-fixture")
            .json(&operation)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let upload = format!("/accounts/{}/attachments", account.id);
    assert_eq!(
        request(reqwest::Method::POST, &upload, "owner-fixture")
            .body("1234567890")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let full = request(reqwest::Method::POST, &upload, "owner-fixture")
        .body("1")
        .send()
        .await
        .unwrap();
    assert_eq!(full.status(), 507);
    assert_eq!(
        full.json::<serde_json::Value>().await.unwrap()["code"],
        "attachment_quota_exceeded"
    );
    // Keep an actual authenticated SSE response open across SIGTERM. It must
    // drain rather than forcing every deployment to wait for Docker's timeout.
    let stream = request(
        reqwest::Method::GET,
        &format!("/accounts/{}/events", account.id),
        "owner-fixture",
    )
    .send()
    .await
    .unwrap();
    assert_eq!(stream.status(), 200);
    assert!(std::process::Command::new("kill")
        .arg("-TERM")
        .arg(child.id().unwrap().to_string())
        .status()
        .unwrap()
        .success());
    let exit = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
        .await
        .expect("open SSE prevented graceful shutdown")
        .unwrap();
    assert!(exit.success());
    assert!(stream.bytes().await.is_ok());
}

#[tokio::test]
async fn runtime_translation_queue_and_mutation_http_boundaries() {
    runtime_contract(None, None).await;
}
#[tokio::test]
#[ignore = "requires disposable dakia_runtime_test database; DAKIA_TEST_RUNTIME_POSTGRES_URL"]
async fn four_backend_native_server_http_contract() {
    let url = std::env::var("DAKIA_TEST_RUNTIME_POSTGRES_URL").unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    assert_eq!(
        options.get_database(),
        Some("dakia_runtime_test"),
        "refuse to reset any other database"
    );
    for (postgres_app, postgres_search) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .unwrap();
        for schema in ["dakia_mail", "dakia_control", "dakia_search"] {
            sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                .execute(&admin)
                .await
                .unwrap();
        }
        admin.close().await;
        let temp = tempfile::tempdir().unwrap();
        let sqlite = format!("sqlite:{}", temp.path().join("search.sqlite").display());
        runtime_contract(
            postgres_app.then_some(url.as_str()),
            Some(if postgres_search {
                url.as_str()
            } else {
                &sqlite
            }),
        )
        .await;
    }
}
