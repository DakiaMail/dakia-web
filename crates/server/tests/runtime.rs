//! Real TCP server smoke with synthetic local state, jobs disabled and no provider I/O.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use dakia_domain::*;
use dakia_storage::Store;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn runtime_translation_queue_and_mutation_http_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let store = Store::open_with_attachment_budget(&data, [8; 32], 10)
        .await
        .unwrap();
    let user = store
        .user("https://id.example.invalid", "owner")
        .await
        .unwrap();
    for (subject, token) in [("owner", "owner-fixture"), ("other", "other-fixture")] {
        let user = store
            .user("https://id.example.invalid", subject)
            .await
            .unwrap();
        sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
            .bind(URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes())))
            .bind(user)
            .bind("fixture-csrf")
            .bind(4102444800_i64)
            .execute(store.control())
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO settings(key,value) VALUES('jobs_enabled','false')")
        .execute(store.control())
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
    let mut child = tokio::process::Command::new(binary)
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
        .env("DAKIA_CLASSIFIER_DIR", dir.path().join("absent-classifier"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
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
    // The real binary must answer from SQLite without resolving the fictional
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
