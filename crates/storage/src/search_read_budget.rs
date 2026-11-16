//! Read-only search work must stop in the database as well as in the caller.
//! Pooled connections are reusable after deadline expiry or future cancellation.
use crate::Result;
use sqlx::{
    sqlite::SqlitePoolOptions, PgConnection, PgPool, Postgres, Sqlite, SqliteConnection,
    SqlitePool, Transaction,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
// Outage safeguard, not a search latency target.
const TURN: std::time::Duration = std::time::Duration::from_secs(10);
pub(super) fn sqlite_pool() -> SqlitePoolOptions {
    crate::sqlite_pool_options().max_connections(4)
}
pub(super) struct SqliteRead {
    connection: sqlx::pool::PoolConnection<sqlx::Sqlite>,
    active: Arc<AtomicBool>,
}
impl SqliteRead {
    pub fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }
}
impl Drop for SqliteRead {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}
pub(super) async fn sqlite(db: &SqlitePool) -> Result<SqliteRead> {
    sqlite_for(db, TURN).await
}
async fn sqlite_for(db: &SqlitePool, turn: std::time::Duration) -> Result<SqliteRead> {
    let mut connection = db.acquire().await?;
    let active = Arc::new(AtomicBool::new(true));
    let alive = active.clone();
    let until = std::time::Instant::now() + turn;
    connection
        .lock_handle()
        .await?
        .set_progress_handler(1000, move || {
            alive.load(Ordering::Acquire) && std::time::Instant::now() < until
        });
    Ok(SqliteRead { connection, active })
}

/// Ranking bounds and their admitted head must observe the same target state.
/// The owned transaction also rolls back when the enclosing search is cancelled.
pub(super) struct SqliteSnapshot {
    transaction: Transaction<'static, Sqlite>,
    active: Arc<AtomicBool>,
}
impl SqliteSnapshot {
    pub fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.transaction
    }
}
impl Drop for SqliteSnapshot {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}
pub(super) async fn sqlite_snapshot(db: &SqlitePool) -> Result<SqliteSnapshot> {
    let mut transaction = db.begin().await?;
    let active = Arc::new(AtomicBool::new(true));
    let alive = active.clone();
    let until = std::time::Instant::now() + TURN;
    transaction
        .lock_handle()
        .await?
        .set_progress_handler(1000, move || {
            alive.load(Ordering::Acquire) && std::time::Instant::now() < until
        });
    Ok(SqliteSnapshot {
        transaction,
        active,
    })
}
pub(super) struct PostgresRead {
    transaction: Transaction<'static, Postgres>,
}
impl PostgresRead {
    pub fn connection(&mut self) -> &mut PgConnection {
        &mut self.transaction
    }
}
pub(super) async fn postgres(db: &PgPool) -> Result<PostgresRead> {
    postgres_for(db, TURN).await
}
async fn postgres_for(db: &PgPool, turn: std::time::Duration) -> Result<PostgresRead> {
    let mut transaction = db.begin().await?;
    sqlx::query(&format!(
        "SET LOCAL statement_timeout='{}ms'",
        turn.as_millis()
    ))
    .execute(&mut *transaction)
    .await?;
    Ok(PostgresRead { transaction })
}
pub(super) async fn postgres_snapshot(db: &PgPool) -> Result<PostgresRead> {
    let mut read = postgres(db).await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(read.connection())
        .await?;
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXPENSIVE:&str="WITH RECURSIVE values_to_sum(n) AS(VALUES(0) UNION ALL SELECT n+1 FROM values_to_sum WHERE n<1000000000) SELECT sum(n) FROM values_to_sum";
    #[tokio::test]
    async fn sqlite_ranking_snapshot_is_stable_and_cancelled_reads_roll_back() {
        let dir = tempfile::tempdir().unwrap();
        let db = sqlite_pool()
            .max_connections(2)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.path().join("snapshot.sqlite"))
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE proof(value INTEGER);INSERT INTO proof VALUES(1)")
            .execute(&db)
            .await
            .unwrap();
        let mut snapshot = sqlite_snapshot(&db).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT value FROM proof")
                .fetch_one(snapshot.connection())
                .await
                .unwrap(),
            1
        );
        sqlx::query("UPDATE proof SET value=2")
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT value FROM proof")
                .fetch_one(snapshot.connection())
                .await
                .unwrap(),
            1
        );
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(10),
            sqlx::query_scalar::<_, i64>(EXPENSIVE).fetch_one(snapshot.connection())
        )
        .await
        .is_err());
        drop(snapshot);
        // Both physical connections must be reusable, with no pinned snapshot.
        let (left, right) = tokio::join!(db.acquire(), db.acquire());
        for mut connection in [left.unwrap(), right.unwrap()] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT value FROM proof")
                    .fetch_one(&mut *connection)
                    .await
                    .unwrap(),
                2
            );
            sqlx::Connection::begin_with(&mut *connection, "BEGIN IMMEDIATE")
                .await
                .unwrap()
                .rollback()
                .await
                .unwrap();
        }
        db.close().await;
    }
    #[tokio::test]
    async fn sqlite_deadlines_and_cancellation_release_workers_without_poisoning_writes() {
        let dir = tempfile::tempdir().unwrap();
        let db = sqlite_pool()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.path().join("budget.sqlite"))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let mut read = sqlite_for(&db, std::time::Duration::from_millis(15))
            .await
            .unwrap();
        assert!(sqlx::query_scalar::<_, i64>(EXPENSIVE)
            .fetch_one(read.connection())
            .await
            .is_err());
        drop(read);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        // Cancel before its own long deadline. Drop signals the native progress
        // callback; the single connection must become available immediately.
        let started = std::time::Instant::now();
        let mut read = sqlite_for(&db, std::time::Duration::from_secs(30))
            .await
            .unwrap();
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(15),
            sqlx::query_scalar::<_, i64>(EXPENSIVE).fetch_one(read.connection())
        )
        .await
        .is_err());
        drop(read);
        tokio::time::timeout(std::time::Duration::from_secs(1),async {
            sqlx::query("CREATE TABLE durable(value INTEGER)").execute(&db).await.unwrap();
            sqlx::query("WITH RECURSIVE numbers(n) AS(VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000) INSERT INTO durable SELECT n FROM numbers").execute(&db).await.unwrap();
            assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM durable").fetch_one(&db).await.unwrap(),10000);
        }).await.unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
    async fn postgres_ranking_snapshot_is_stable_and_cancelled_reads_roll_back() {
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap())
            .await
            .unwrap();
        let table = format!("dakia_snapshot_{}", uuid::Uuid::new_v4().simple());
        sqlx::raw_sql(&format!(
            "CREATE TABLE {table}(value INTEGER);INSERT INTO {table} VALUES(1)"
        ))
        .execute(&db)
        .await
        .unwrap();
        let sql = format!("SELECT CAST(value AS BIGINT) FROM {table}");
        let mut snapshot = postgres_snapshot(&db).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&sql)
                .fetch_one(snapshot.connection())
                .await
                .unwrap(),
            1
        );
        sqlx::query(&format!("UPDATE {table} SET value=2"))
            .execute(&db)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&sql)
                .fetch_one(snapshot.connection())
                .await
                .unwrap(),
            1
        );
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(10),
            sqlx::query("SELECT pg_sleep(10)").execute(snapshot.connection())
        )
        .await
        .is_err());
        drop(snapshot);
        // PostgreSQL finishes a cancelled in-flight statement at its scoped deadline
        // before the queued rollback returns the connection to the pool.
        let (left, right) = tokio::time::timeout(TURN + std::time::Duration::from_secs(2), async {
            tokio::join!(db.acquire(), db.acquire())
        })
        .await
        .unwrap();
        for mut connection in [left.unwrap(), right.unwrap()] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&sql)
                    .fetch_one(&mut *connection)
                    .await
                    .unwrap(),
                2
            );
        }
        sqlx::query(&format!("DROP TABLE {table}"))
            .execute(&db)
            .await
            .unwrap();
        db.close().await;
    }
    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL; DAKIA_TEST_POSTGRES_URL"]
    async fn postgres_statement_deadline_rolls_back_without_poisoning_the_pool() {
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap())
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let mut read = postgres_for(&db, std::time::Duration::from_millis(20))
            .await
            .unwrap();
        assert!(sqlx::query("SELECT pg_sleep(10)")
            .execute(read.connection())
            .await
            .is_err());
        drop(read);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        // This is longer than the scoped budget and must still succeed.
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            sqlx::query("SELECT pg_sleep(1.0)").execute(&db),
        )
        .await
        .unwrap()
        .unwrap();
        let mut read = postgres_for(&db, std::time::Duration::from_millis(20))
            .await
            .unwrap();
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(15),
            sqlx::query("SELECT pg_sleep(10)").execute(read.connection())
        )
        .await
        .is_err());
        drop(read);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            sqlx::query("SELECT 1").execute(&db),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT 1::BIGINT")
                .fetch_one(&db)
                .await
                .unwrap(),
            1
        );
        db.close().await;
    }
}
