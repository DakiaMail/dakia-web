//! Compatibility for offline Store callers. The production server's independent
//! search repository must not leave FTS triggers on authoritative mailbox writes.
use super::*;

pub(super) async fn prepare(db: &SqlitePool, external: bool) -> Result<()> {
    // Main and the search branch both used 24. Recognize only main's exact migration.
    let ledger: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='_sqlx_migrations')",
    )
    .fetch_one(db)
    .await?;
    if ledger {
        let migrations = sqlx::migrate!("./migrations/account");
        let attachment = migrations
            .iter()
            .find(|m| m.version == 41)
            .ok_or(Error::Invalid)?;
        sqlx::query(
            "UPDATE _sqlx_migrations SET version=41 WHERE version=24 AND checksum=? AND success=1",
        )
        .bind(attachment.checksum.as_ref())
        .execute(db)
        .await?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upgrades_both_main_and_search_branch_migration_24() {
        for from_main in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let db = crate::pool(&dir.path().join("account.sqlite"))
                .await
                .unwrap();
            let migrations = sqlx::migrate!("./migrations/account");
            let mut old: Vec<_> = migrations
                .iter()
                .filter(|m| m.version <= if from_main { 23 } else { 40 })
                .cloned()
                .collect();
            if from_main {
                let mut attachment = migrations.iter().find(|m| m.version == 41).unwrap().clone();
                attachment.version = 24;
                old.push(attachment);
            }
            sqlx::migrate::Migrator {
                migrations: std::borrow::Cow::Owned(old),
                ..sqlx::migrate::Migrator::DEFAULT
            }
            .run(&db)
            .await
            .unwrap();
            let message = r#"{"id":"retained","account_id":"fixture","thread_id":"retained","provider_id":"retained","folder_ids":["INBOX"],"sender":{"email":"fixture@example.invalid","name":null},"to":[],"subject":"Upgrade fixture","preview":"","received_at":"2026-01-01T00:00:00Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":1}"#;
            sqlx::query("INSERT INTO messages(id,revision,document) VALUES('retained',1,?)")
                .bind(message)
                .execute(&db)
                .await
                .unwrap();
            prepare(&db, true).await.unwrap();
            migrations.run(&db).await.unwrap();
            prepare(&db, true).await.unwrap();
            migrations.run(&db).await.unwrap();
            let fields: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_table_info('search_documents') WHERE name IN ('attachment_text','has_attachment')").fetch_one(&db).await.unwrap();
            assert_eq!(fields, 2);
            assert_eq!(
                sqlx::query_scalar::<_, String>(
                    "SELECT document FROM messages WHERE id='retained'"
                )
                .fetch_one(&db)
                .await
                .unwrap(),
                message
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM _sqlx_migrations WHERE version IN (24,41,42)"
                )
                .fetch_one(&db)
                .await
                .unwrap(),
                3
            );
        }
    }
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
