//! Run the shipped operator binary against synthetic data and an isolated DB.
use dakia_domain::*;
use dakia_storage::Store;
async fn command(
    action: &str,
    source: &std::path::Path,
    target: &std::path::Path,
    key: &std::path::Path,
    database: Option<&str>,
) -> std::process::Output {
    let binary = std::env::var("DAKIA_TEST_ADMIN_BINARY")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_dakia-admin").into());
    tokio::process::Command::new(binary)
        .arg(action)
        .arg(source)
        .arg(target)
        .arg(key)
        .env("DAKIA_DATABASE_URL", database.unwrap_or(""))
        .output()
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires disposable dakia_operator_test database; DAKIA_TEST_OPERATOR_POSTGRES_URL"]
async fn native_operator_backend_transfer_roundtrip() {
    let url = std::env::var("DAKIA_TEST_OPERATOR_POSTGRES_URL").unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    assert_eq!(
        options.get_database(),
        Some("dakia_operator_test"),
        "refuse to reset any other database"
    );
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for schema in ["dakia_mail", "dakia_control", "dakia_search"] {
        sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key");
    std::fs::write(&key, [23; 32]).unwrap();
    let source = dir.path().join("source");
    let store = Store::open(&source, [23; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let input:ConnectAccount=serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let draft:Draft=serde_json::from_value(serde_json::json!({"id":"operator-draft","account_id":account.id,"revision":0,"to":[{"email":"recipient@example.invalid","name":null}],"cc":[],"bcc":[],"subject":"Durable","text":"Body","html":"","in_reply_to":null,"references":[],"attachments":[],"saved_at":""})).unwrap();
    let draft = store.save_draft(&a, draft).await.unwrap();
    let request = SubmitRequest {
        operation_id: "operator-submit".into(),
        draft_id: draft.id.clone(),
        expected_revision: draft.revision,
        due_at: Some("2099-01-01T00:00:00Z".into()),
        timezone: "UTC".into(),
    };
    let submission = store.submit(&a, &request).await.unwrap();
    drop(store);
    let archive = dir.path().join("archive");
    let result = command("export", &source, &archive, &key, None).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let postgres = dir.path().join("postgres");
    let result = command("import", &archive, &postgres, &key, Some(&url)).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let live = Store::open_with_database(&postgres, [23; 32], 1024, Some(&url))
        .await
        .unwrap();
    let live_a = live.authorize(&user, &account.id).await.unwrap();
    assert_eq!(
        live.credential(&live_a).await.unwrap().password,
        input.password
    );
    assert_eq!(live.submit(&live_a, &request).await.unwrap(), submission);
    let blocked = command(
        "export",
        &postgres,
        &dir.path().join("blocked"),
        &key,
        Some(&url),
    )
    .await;
    assert!(!blocked.status.success());
    assert_eq!(live.draft(&live_a, &draft.id).await.unwrap(), draft);
    drop(live);
    let pg_archive = dir.path().join("postgres-archive");
    let result = command("export", &postgres, &pg_archive, &key, Some(&url)).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let final_root = dir.path().join("roundtrip");
    let result = command("import", &pg_archive, &final_root, &key, None).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Store::open(&final_root, [23; 32]).await.unwrap();
    let final_a = result.authorize(&user, &account.id).await.unwrap();
    assert_eq!(
        result.credential(&final_a).await.unwrap().password,
        input.password
    );
    assert_eq!(result.draft(&final_a, &draft.id).await.unwrap(), draft);
    assert_eq!(result.submit(&final_a, &request).await.unwrap(), submission);
    assert!(!result.jobs_enabled().await.unwrap());
}
