//! Opt-in isolated benchmark through production IMAP reconciliation and SQLite.
use super::*;
use p::MailProvider;
use std::time::Instant;

#[tokio::test]
#[ignore = "requires disposable synthetic IMAP fixture; see benchmarks/initial-sync"]
async fn initial_mailbox_sync_benchmark() {
    let count: i64 = std::env::var("BENCH_COUNT")
        .unwrap_or_else(|_| "50000".into())
        .parse()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [53; 32]).await.unwrap();
    let user = store.user("benchmark", "synthetic").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.test","display_name":"Benchmark","provider":"imap","imap_host":"localhost","imap_port":1993,"imap_tls":"implicit","smtp_host":"localhost","smtp_port":1465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only-password"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    store.account_state(&a, AccountState::Ready).await.unwrap();
    let endpoint = p::Endpoint {
        host: "localhost".into(),
        port: std::env::var("BENCH_PORT")
            .unwrap_or_else(|_| "1993".into())
            .parse()
            .unwrap(),
        security: p::Security::Tls,
    };
    let provider = Arc::new(
        p::AccountProvider::new(
            p::Config {
                account_id: a.id().into(),
                primary_address: "fixture@example.test".into(),
                kind: p::ProviderKind::Imap,
                imap: endpoint.clone(),
                smtp: endpoint,
                timeout: Duration::from_secs(30),
                full_imap_exposure_confirmed: true,
                folders: p::FolderMapping::default(),
            },
            p::Credentials {
                username: "fixture".into(),
                password: "fixture-only-password".into(),
            },
        )
        .unwrap()
        .with_imap_root(
            native_tls::Certificate::from_pem(
                &std::fs::read(std::env::var("BENCH_CERT").unwrap()).unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let runtime = MailRuntime::default();
    runtime
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    let app = crate::App {
        store,
        mail: runtime,
        config: Arc::new(crate::Config {
            origin: "https://benchmark.invalid".into(),
            issuer: "https://benchmark.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture".into(),
            web_root: dir.path().to_string_lossy().into(),
        }),
    };
    let scheduled = std::env::var("BENCH_SCHEDULED").as_deref() == Ok("1");
    let start = Instant::now();
    provider.folders().await.unwrap();
    let mut turns = 0;
    let worker = scheduled.then(|| tokio::spawn(crate::executor::run(app.clone())));
    loop {
        if scheduled {
            tokio::time::sleep(Duration::from_millis(100)).await;
        } else {
            crate::sync::tick(&app, &a).await.unwrap();
            crate::sync::full_text::tick(&app, &a).await.unwrap();
        }
        turns += 1;
        if app
            .store
            .terminal_scan_folders(&a)
            .await
            .unwrap()
            .iter()
            .any(|f| f == "INBOX")
        {
            let coverage = app.store.text_coverage(&a).await.unwrap();
            if coverage.bodies == count
                && coverage.headers == count
                && coverage.dirty == 0
                && coverage.gaps == 0
            {
                break;
            }
        }
        assert!(
            start.elapsed()
                < Duration::from_secs_f64(
                    std::env::var("BENCH_MAX_SECONDS")
                        .unwrap_or_else(|_| "3600".into())
                        .parse()
                        .unwrap()
                )
        );
        if turns % 100 == 0 {
            eprintln!("turns={turns} elapsed={:.3}", start.elapsed().as_secs_f64());
        }
    }
    let seconds = start.elapsed().as_secs_f64();
    if let Some(worker) = worker {
        worker.abort();
        let _ = worker.await;
    }
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(rows, count);
    let members: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM folder_memberships WHERE folder='INBOX'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(members, count);
    let uids: (i64, i64, i64) = sqlx::query_as("SELECT COUNT(DISTINCT json_extract(json_extract(document,'$.provider_id'),'$.uid')), MIN(json_extract(json_extract(document,'$.provider_id'),'$.uid')), MAX(json_extract(json_extract(document,'$.provider_id'),'$.uid')) FROM messages").fetch_one(&db).await.unwrap();
    assert_eq!(uids, (count, 1, count));
    let invalid: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE json_extract(document,'$.subject') <> 'Benchmark message ' || json_extract(json_extract(document,'$.provider_id'),'$.uid') OR json_extract(document,'$.unread') <> 1").fetch_one(&db).await.unwrap();
    assert_eq!(
        invalid, 0,
        "persisted headers must match their UID and stay unread"
    );
    let indexed: i64 = sqlx::query_scalar(r#"SELECT COUNT(*) FROM search_fts WHERE search_fts MATCH 'body:"deterministic mailbox benchmark"'"#).fetch_one(&db).await.unwrap();
    assert_eq!(indexed, count);
    let wrong_body: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_documents d JOIN messages m ON m.id=d.message_id WHERE instr(d.body,'search token body' || json_extract(json_extract(m.document,'$.provider_id'),'$.uid') || '.')=0 OR d.body_state<>'ready'").fetch_one(&db).await.unwrap();
    assert_eq!(wrong_body, 0);
    println!(
        "INITIAL_SYNC {}",
        serde_json::json!({"client":"dakia","seconds":seconds,"rows":rows,"indexed_bodies":indexed,"verified_body_tokens":count-wrong_body,"memberships":members,"turns_or_polls":turns,"scheduled":scheduled,"milestone":"all bodies downloaded and indexed; metadata and membership reconciled"})
    );
    provider.retire();
}
