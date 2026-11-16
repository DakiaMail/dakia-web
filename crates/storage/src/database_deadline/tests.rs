use super::*;
use crate::Result;

const EXPENSIVE: &str = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) INSERT INTO work SELECT x FROM n";

#[tokio::test]
async fn sqlite_expiry_and_cancellation_release_fresh_and_reused_connections() {
    for fresh in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("deadline.sqlite"))
            .create_if_missing(true);
        let db = crate::sqlite_pool_options().connect_lazy_with(options);
        if !fresh {
            sqlx::query("SELECT 1").execute(&db).await.unwrap();
        }
        let result: Result<()> = storage_deadline(Duration::from_millis(25), async {
            sqlx::query("CREATE TABLE work(value INTEGER)")
                .execute(&db)
                .await?;
            let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
            sqlx::query(EXPENSIVE).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(())
        })
        .await;
        assert!(matches!(result, Err(Error::DeadlineExceeded)));
        tokio::time::timeout(Duration::from_secs(1), async {
            sqlx::query("CREATE TABLE IF NOT EXISTS work(value INTEGER)")
                .execute(&db)
                .await
                .unwrap();
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM work")
                    .fetch_one(&db)
                    .await
                    .unwrap(),
                0
            );
        })
        .await
        .unwrap();
        let work = storage_deadline(Duration::from_secs(30), async {
            let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
            sqlx::query(EXPENSIVE).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok::<_, Error>(())
        });
        assert!(tokio::time::timeout(Duration::from_millis(25), work)
            .await
            .is_err());
        tokio::time::timeout(Duration::from_secs(1), async {
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM work")
                    .fetch_one(&db)
                    .await
                    .unwrap(),
                0
            );
            sqlx::query("INSERT INTO work VALUES(7)")
                .execute(&db)
                .await
                .unwrap();
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT sum(value) FROM work")
                    .fetch_one(&db)
                    .await
                    .unwrap(),
                7
            );
        })
        .await
        .unwrap();
        db.close().await;
    }
}

#[tokio::test]
async fn nested_deadlines_cannot_extend_the_parent() {
    let start = Instant::now();
    let result: Result<()> = storage_deadline(Duration::from_millis(25), async {
        storage_deadline(Duration::from_secs(10), async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(())
        })
        .await
    })
    .await;
    assert!(matches!(result, Err(Error::DeadlineExceeded)));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(CURRENT.try_with(|_| ()).is_err());
}

#[tokio::test]
async fn sqlite_lock_waits_cancel_and_restore_operator_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.path().join("busy.sqlite"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_millis(750));
    let db = crate::sqlite_pool_options()
        .connect_with(options.clone())
        .await
        .unwrap();
    sqlx::query("CREATE TABLE work(value INTEGER)")
        .execute(&db)
        .await
        .unwrap();
    let locker = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let lock = locker.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let result: Result<()> = storage_deadline(Duration::from_millis(25), async {
        db.begin_with("BEGIN IMMEDIATE").await?.rollback().await?;
        Ok(())
    })
    .await;
    assert!(matches!(result, Err(Error::DeadlineExceeded)));
    tokio::time::timeout(Duration::from_millis(250), db.acquire())
        .await
        .unwrap()
        .unwrap();
    let work = storage_deadline(Duration::from_secs(10), async {
        db.begin_with("BEGIN IMMEDIATE").await?.rollback().await?;
        Ok::<_, Error>(())
    });
    assert!(tokio::time::timeout(Duration::from_millis(25), work)
        .await
        .is_err());
    tokio::time::timeout(Duration::from_millis(250), db.acquire())
        .await
        .unwrap()
        .unwrap();
    lock.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i32>("PRAGMA busy_timeout")
            .fetch_one(&db)
            .await
            .unwrap(),
        750
    );
    db.begin_with("BEGIN IMMEDIATE")
        .await
        .unwrap()
        .rollback()
        .await
        .unwrap();
    db.close().await;
    locker.close().await;
}

#[tokio::test]
#[ignore = "requires isolated dakia_timeout_test PostgreSQL; DAKIA_TEST_DEADLINE_POSTGRES_URL"]
async fn postgres_deadlines_bound_queries_locks_and_restore_operator_settings() {
    use sqlx::Connection;
    let url = std::env::var("DAKIA_TEST_DEADLINE_POSTGRES_URL").unwrap();
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    assert_eq!(options.get_database(), Some("dakia_timeout_test"));
    let options = options.options([("statement_timeout", "500ms"), ("lock_timeout", "400ms")]);
    for fresh in [true, false] {
        let db = postgres_pool_options(&options)
            .max_connections(1)
            .connect_lazy_with(options.clone());
        if !fresh {
            sqlx::query("SELECT 1").execute(&db).await.unwrap();
        }
        let started = Instant::now();
        let result: Result<()> = storage_deadline(Duration::from_millis(75), async {
            sqlx::query("SELECT pg_sleep(5)").execute(&db).await?;
            Ok(())
        })
        .await;
        assert!(matches!(result, Err(Error::DeadlineExceeded)));
        tokio::time::timeout(Duration::from_millis(250), async {
            let (statement, lock): (String, String) = sqlx::query_as(
                "SELECT current_setting('statement_timeout'),current_setting('lock_timeout')",
            )
            .fetch_one(&db)
            .await
            .unwrap();
            assert_eq!((statement.as_str(), lock.as_str()), ("500ms", "400ms"));
        })
        .await
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        let result: Result<()> = storage_deadline(Duration::from_secs(2), async {
            let mut tx = db.begin().await?;
            postgres_read_limit(&mut tx, Duration::from_secs(10)).await?;
            let (statement, lock): (String, String) = sqlx::query_as(
                "SELECT current_setting('statement_timeout'),current_setting('lock_timeout')",
            )
            .fetch_one(&mut *tx)
            .await?;
            assert_eq!((statement.as_str(), lock.as_str()), ("500ms", "400ms"));
            tx.rollback().await?;
            Ok(())
        })
        .await;
        result.unwrap();
        let result: Result<()> = storage_deadline(Duration::from_millis(75), async {
            let mut tx = db.begin().await?;
            postgres_read_limit(&mut tx, Duration::from_secs(10)).await?;
            sqlx::query("SELECT pg_sleep(5)").execute(&mut *tx).await?;
            Ok(())
        })
        .await;
        assert!(matches!(result, Err(Error::DeadlineExceeded)));
        tokio::time::timeout(Duration::from_millis(250), db.acquire())
            .await
            .unwrap()
            .unwrap();
        let work = storage_deadline(Duration::from_millis(150), async {
            sqlx::query("SELECT pg_sleep(5)").execute(&db).await?;
            Ok::<_, Error>(())
        });
        assert!(tokio::time::timeout(Duration::from_millis(25), work)
            .await
            .is_err());
        tokio::time::timeout(Duration::from_millis(350), async {
            sqlx::query("SELECT 1").execute(&db).await.unwrap();
        })
        .await
        .unwrap();

        let table = format!("deadline_{}", uuid::Uuid::new_v4().simple());
        let mut locker = sqlx::PgConnection::connect_with(&options).await.unwrap();
        sqlx::raw_sql(&format!(
            "CREATE TABLE {table}(value INTEGER); INSERT INTO {table} VALUES(1)"
        ))
        .execute(&mut locker)
        .await
        .unwrap();
        let mut lock = locker.begin().await.unwrap();
        sqlx::query(&format!("UPDATE {table} SET value=2"))
            .execute(&mut *lock)
            .await
            .unwrap();
        let result: Result<()> = storage_deadline(Duration::from_millis(75), async {
            let mut tx = db.begin().await?;
            sqlx::query(&format!("UPDATE {table} SET value=3"))
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(())
        })
        .await;
        assert!(matches!(result, Err(Error::DeadlineExceeded)));
        lock.rollback().await.unwrap();
        tokio::time::timeout(Duration::from_millis(250), async {
            assert_eq!(
                sqlx::query_scalar::<_, i32>(&format!("SELECT value FROM {table}"))
                    .fetch_one(&db)
                    .await
                    .unwrap(),
                1
            );
        })
        .await
        .unwrap();
        sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&mut locker)
            .await
            .unwrap();
        db.close().await;
    }
}

#[tokio::test]
#[ignore = "requires isolated dakia_timeout_test PostgreSQL; DAKIA_TEST_DEADLINE_POSTGRES_URL"]
async fn postgres_absolute_scope_cancels_late_statement_and_fences_reuse() {
    use sqlx::Connection;
    let options: sqlx::postgres::PgConnectOptions =
        std::env::var("DAKIA_TEST_DEADLINE_POSTGRES_URL")
            .unwrap()
            .parse()
            .unwrap();
    assert_eq!(options.get_database(), Some("dakia_timeout_test"));
    let db = postgres_pool_options(&options)
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    let table = format!("deadline_late_{}", uuid::Uuid::new_v4().simple());
    let mut observer = PgConnection::connect_with(&options).await.unwrap();
    sqlx::raw_sql(&format!(
        "CREATE TABLE {table}(value INTEGER); INSERT INTO {table} VALUES(1)"
    ))
    .execute(&mut observer)
    .await
    .unwrap();
    let started = Instant::now();
    let result: Result<()> = storage_deadline(Duration::from_millis(400), async {
        let mut tx = db.begin().await?;
        sqlx::query(&format!("UPDATE {table} SET value=2"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_sleep(0.3)")
            .execute(&mut *tx)
            .await?;
        // The checkout statement cap alone would retain this transaction's row lock to700ms.
        sqlx::query("SELECT pg_sleep(5)").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    })
    .await;
    assert!(matches!(result, Err(Error::DeadlineExceeded)));
    tokio::time::timeout(
        Duration::from_millis(200),
        sqlx::query(&format!("UPDATE {table} SET value=3")).execute(&mut observer),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(started.elapsed() < Duration::from_millis(600));
    // Successful scope release must retire its timer before the same backend is lent again.
    let first: Result<i32> = storage_deadline(Duration::from_millis(100), async {
        Ok(sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&db)
            .await?)
    })
    .await;
    let first = first.unwrap();
    let mut connection = db.acquire().await.unwrap();
    let second: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!(first, second);
    sqlx::query("SELECT pg_sleep(0.2)")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    sqlx::query(&format!("DROP TABLE {table}"))
        .execute(&mut observer)
        .await
        .unwrap();
    db.close().await;
}
