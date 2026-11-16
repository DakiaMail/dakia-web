//! Ignored synthetic-fixture diagnostics, never production performance acceptance.
use super::*;

async fn driver_counters(connection: &mut sqlx::SqliteConnection) -> serde_json::Value {
    let mut handle = connection.lock_handle().await.unwrap();
    // The SQLx guard excludes its worker while these read-only diagnostic
    // APIs inspect the owned connection and its cached statements.
    unsafe {
        let db = handle.as_raw_handle().as_ptr();
        let mut counters = serde_json::Map::new();
        for (name, code) in [
            ("cache_hits", libsqlite3_sys::SQLITE_DBSTATUS_CACHE_HIT),
            ("cache_misses", libsqlite3_sys::SQLITE_DBSTATUS_CACHE_MISS),
            ("cache_bytes", libsqlite3_sys::SQLITE_DBSTATUS_CACHE_USED),
        ] {
            let (mut current, mut high) = (0, 0);
            assert_eq!(
                libsqlite3_sys::sqlite3_db_status(db, code, &mut current, &mut high, 0),
                0
            );
            counters.insert(name.into(), current.into());
        }
        let (mut vm, mut scans, mut sort, mut reprepare) = (0_i64, 0_i64, 0_i64, 0_i64);
        let mut statement = libsqlite3_sys::sqlite3_next_stmt(db, std::ptr::null_mut());
        while !statement.is_null() {
            vm += i64::from(libsqlite3_sys::sqlite3_stmt_status(
                statement,
                libsqlite3_sys::SQLITE_STMTSTATUS_VM_STEP,
                0,
            ));
            scans += i64::from(libsqlite3_sys::sqlite3_stmt_status(
                statement,
                libsqlite3_sys::SQLITE_STMTSTATUS_FULLSCAN_STEP,
                0,
            ));
            sort += i64::from(libsqlite3_sys::sqlite3_stmt_status(
                statement,
                libsqlite3_sys::SQLITE_STMTSTATUS_SORT,
                0,
            ));
            reprepare += i64::from(libsqlite3_sys::sqlite3_stmt_status(
                statement,
                libsqlite3_sys::SQLITE_STMTSTATUS_REPREPARE,
                0,
            ));
            statement = libsqlite3_sys::sqlite3_next_stmt(db, statement);
        }
        counters.insert("vm_steps".into(), vm.into());
        counters.insert("fullscan_steps".into(), scans.into());
        counters.insert("sorts".into(), sort.into());
        counters.insert("reprepares".into(), reprepare.into());
        serde_json::Value::Object(counters)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "synthetic retained-fixture driver diagnostic only; not latency acceptance"]
async fn sqlite_score_head_driver_diagnostic() {
    assert!(!cfg!(debug_assertions), "use --release");
    let _ = tracing_subscriber::fmt()
        .with_env_filter("dakia_server::search_timing=debug")
        .try_init();
    let root = std::path::PathBuf::from(std::env::var("DAKIA_BENCH_REUSE_ROOT").unwrap());
    assert_eq!(
        std::fs::read(root.join("credential-key")).unwrap(),
        [27; 32]
    );
    let target = SearchDatabase::open(&root.join("scale-app"), None)
        .await
        .unwrap();
    let SearchDatabase::Sqlite(db) = &target else {
        unreachable!()
    };
    let account: String =
        sqlx::query_scalar("SELECT account_id FROM lexical_accounts ORDER BY account_id LIMIT 1")
            .fetch_one(db)
            .await
            .unwrap();
    const SQL:&str="SELECT d.metadata,d.source,CAST(? AS REAL) AS score FROM documents d INDEXED BY documents_filter_order WHERE d.account_id=? AND d.state<>'deleted' AND EXISTS(SELECT 1 FROM lexical_chunks c INDEXED BY lexical_subject_head WHERE c.account_id=d.account_id AND c.message_id=d.message_id AND c.content_version=d.content_version AND c.revision=? AND c.native_text<>'' AND c.field IN (2) AND ((c.field=2 AND instr(' '||c.text||' ',?)>0))) ORDER BY d.received_at DESC,d.message_id DESC LIMIT ?";
    let sql = SQL;
    for round in 0..2 {
        let at = std::time::Instant::now();
        let rows = sqlx::query_as::<_, (String, String, f64)>(sql)
            .bind(-5.)
            .bind(&account)
            .bind(crate::lexical_text::REVISION)
            .bind(" wgarden ")
            .bind(513_i64)
            .fetch_all(db)
            .await
            .unwrap();
        println!(
            "DRIVER_UNGUARDED round={round} ms={:.3} rows={}",
            at.elapsed().as_secs_f64() * 1000.,
            rows.len()
        );
        let at = std::time::Instant::now();
        let rows = single_unit_head(
            &target,
            &account,
            &["wgarden".into()],
            "garden",
            false,
            513,
            None,
        )
        .await;
        println!(
            "DRIVER_SNAPSHOT round={round} ms={:.3} outcome={:?}",
            at.elapsed().as_secs_f64() * 1000.,
            rows.map(|rows| rows.map(|rows| rows.len()))
        );
    }
    let lane = target.read_lane().await.unwrap();
    if let Ok(kib) = std::env::var("DAKIA_DIAGNOSTIC_RANK_CACHE_KIB") {
        let kib: i64 = kib.parse().unwrap();
        assert!((2048..=65536).contains(&kib));
        let SearchDatabase::Sqlite(pool) = &lane else {
            unreachable!()
        };
        let mut held = vec![];
        for _ in 0..pool.options().get_max_connections() {
            held.push(pool.acquire().await.unwrap());
        }
        for connection in &mut held {
            sqlx::query(&format!("PRAGMA cache_size=-{kib}"))
                .execute(&mut **connection)
                .await
                .unwrap();
        }
        println!("DRIVER_RANK_CACHE_KIB={kib}");
    }
    for round in 0..3 {
        let at = std::time::Instant::now();
        let rows = single_unit_head(
            &lane,
            &account,
            &["wgarden".into()],
            "garden",
            false,
            513,
            None,
        )
        .await;
        println!(
            "DRIVER_READ_LANE round={round} ms={:.3} outcome={:?}",
            at.elapsed().as_secs_f64() * 1000.,
            rows.map(|rows| rows.map(|rows| rows.len()))
        );
    }
    if std::env::var_os("DAKIA_DIAGNOSTIC_LOAD_VECTORS").is_some() {
        let accelerator = crate::search_vectors::VectorAccelerator::new(&root.join("scale-app"));
        let at = std::time::Instant::now();
        while accelerator.advance(db).await.unwrap() {}
        println!(
            "DRIVER_VECTOR_RESTORE ms={:.3}",
            at.elapsed().as_secs_f64() * 1000.
        );
        if std::env::var_os("DAKIA_DIAGNOSTIC_SAME_CONNECTION").is_some() {
            let SearchDatabase::Sqlite(pool) = &lane else {
                unreachable!()
            };
            let mut held = vec![];
            for _ in 0..pool.options().get_max_connections() {
                held.push(pool.acquire().await.unwrap());
            }
            for (number, connection) in held.iter_mut().enumerate() {
                for round in 0..2 {
                    let guarded = (number + round) % 2 == 0;
                    let before = driver_counters(connection).await;
                    sqlx::query("BEGIN")
                        .execute(&mut **connection)
                        .await
                        .unwrap();
                    if guarded {
                        let until =
                            std::time::Instant::now() + std::time::Duration::from_millis(800);
                        connection
                            .lock_handle()
                            .await
                            .unwrap()
                            .set_progress_handler(1000, move || std::time::Instant::now() < until);
                    }
                    let at = std::time::Instant::now();
                    let rows = sqlx::query_as::<_, (String, String, f64)>(sql)
                        .bind(-5.)
                        .bind(&account)
                        .bind(crate::lexical_text::REVISION)
                        .bind(" wgarden ")
                        .bind(513_i64)
                        .fetch_all(&mut **connection)
                        .await;
                    println!("DRIVER_SAME_CONNECTION connection={number} round={round} guarded={guarded} ms={:.3} outcome={:?}", at.elapsed().as_secs_f64()*1000., rows.map(|rows| rows.len()));
                    connection
                        .lock_handle()
                        .await
                        .unwrap()
                        .remove_progress_handler();
                    let after = driver_counters(connection).await;
                    println!("DRIVER_NATIVE_COUNTERS connection={number} round={round} before={before} after={after}");
                    // An interrupted read can automatically roll back.
                    let _ = sqlx::query("ROLLBACK").execute(&mut **connection).await;
                }
            }
            return;
        }
        for round in 0..3 {
            let at = std::time::Instant::now();
            let rows = single_unit_head(
                &lane,
                &account,
                &["wgarden".into()],
                "garden",
                false,
                513,
                None,
            )
            .await;
            println!(
                "DRIVER_RESIDENT_VECTORS round={round} ms={:.3} outcome={:?}",
                at.elapsed().as_secs_f64() * 1000.,
                rows.map(|rows| rows.map(|rows| rows.len()))
            );
        }
        let at = std::time::Instant::now();
        let rows = sqlx::query_as::<_, (String, String, f64)>(sql)
            .bind(-5.)
            .bind(&account)
            .bind(crate::lexical_text::REVISION)
            .bind(" wgarden ")
            .bind(513_i64)
            .fetch_all(db)
            .await
            .unwrap();
        println!(
            "DRIVER_RESIDENT_UNGUARDED ms={:.3} rows={}",
            at.elapsed().as_secs_f64() * 1000.,
            rows.len()
        );
        let fresh = SearchDatabase::open(&root.join("scale-app"), None)
            .await
            .unwrap();
        let at = std::time::Instant::now();
        let rows = single_unit_head(
            &fresh,
            &account,
            &["wgarden".into()],
            "garden",
            false,
            513,
            None,
        )
        .await;
        println!(
            "DRIVER_RESIDENT_FRESH_POOL ms={:.3} outcome={:?}",
            at.elapsed().as_secs_f64() * 1000.,
            rows.map(|rows| rows.map(|rows| rows.len()))
        );
        drop(accelerator);
        for round in 0..2 {
            let at = std::time::Instant::now();
            let rows = single_unit_head(
                &lane,
                &account,
                &["wgarden".into()],
                "garden",
                false,
                513,
                None,
            )
            .await;
            println!(
                "DRIVER_AFTER_DROP round={round} ms={:.3} outcome={:?}",
                at.elapsed().as_secs_f64() * 1000.,
                rows.map(|rows| rows.map(|rows| rows.len()))
            );
        }
    }
}
