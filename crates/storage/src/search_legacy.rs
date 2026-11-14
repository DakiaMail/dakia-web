//! Compatibility for offline Store callers. The production server's independent
//! search repository must not leave FTS triggers on authoritative mailbox writes.
use super::*;

pub(super) async fn prepare(db: &SqlitePool, external: bool) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS search_runtime_mode(id INTEGER PRIMARY KEY CHECK(id=1),external INTEGER NOT NULL DEFAULT 0,legacy_ready INTEGER NOT NULL DEFAULT 0)").execute(db).await?;
    sqlx::query("INSERT INTO search_runtime_mode(id,external) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET external=excluded.external WHERE external<>excluded.external").bind(external).execute(db).await?;
    if external {
        let old: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger' AND name='search_delete') AND NOT EXISTS(SELECT 1 FROM pragma_table_info('search_documents') WHERE name='attachment_text')")
            .fetch_one(db).await?;
        if old {
            // Migration 24 removes excluded text before migration 26 replaces the old index.
            let mut tx = db.begin().await?;
            sqlx::Executor::execute(&mut *tx, "DROP TRIGGER search_delete;
                CREATE TRIGGER search_delete AFTER DELETE ON search_documents BEGIN
                    UPDATE search_totals SET body_bytes=body_bytes-length(CAST(old.body AS BLOB)) WHERE id=1;
                END;").await?;
            tx.commit().await?;
        }
    }
    Ok(())
}

pub(super) async fn configure(db: &SqlitePool, external: bool) -> Result<()> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='search_fts' AND type='table')",
    )
    .fetch_one(db)
    .await?;
    let ready: bool = sqlx::query_scalar("SELECT legacy_ready FROM search_runtime_mode WHERE id=1")
        .fetch_one(db)
        .await?;
    if !external && exists && ready {
        return Ok(());
    }
    let mut tx = db.begin().await?;
    for name in [
        "search_fts_insert",
        "search_fts_delete",
        "search_fts_update",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {name}"))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DROP TABLE IF EXISTS search_fts")
        .execute(&mut *tx)
        .await?;
    if !external {
        sqlx::Executor::execute(&mut *tx, include_str!("search_legacy.sql")).await?;
    }
    sqlx::query("UPDATE search_runtime_mode SET legacy_ready=? WHERE id=1")
        .bind(!external)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
