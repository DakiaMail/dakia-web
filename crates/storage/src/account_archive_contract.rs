use super::*;
use sqlx::{Column, Row, TypeInfo, ValueRef};
fn pg_row(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let mut values = serde_json::Map::new();
    for (n, col) in row.columns().iter().enumerate() {
        let value = if row.try_get_raw(n).unwrap().is_null() {
            serde_json::Value::Null
        } else {
            match col.type_info().name() {
                "TEXT" => serde_json::json!(row.get::<String, _>(n)),
                "INT8" => serde_json::json!(row.get::<i64, _>(n)),
                "BOOL" => serde_json::json!(i64::from(row.get::<bool, _>(n))),
                "BYTEA" => serde_json::json!(row.get::<Vec<u8>, _>(n)),
                kind => panic!("uncovered native archive type {kind}"),
            }
        };
        values.insert(col.name().into(), value);
    }
    values.into()
}
fn sqlite_row(row: &sqlx::sqlite::SqliteRow) -> serde_json::Value {
    let mut values = serde_json::Map::new();
    for (n, col) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(n).unwrap();
        let value = if raw.is_null() {
            serde_json::Value::Null
        } else {
            match raw.type_info().name() {
                "TEXT" => serde_json::json!(row.get::<String, _>(n)),
                "INTEGER" => serde_json::json!(row.get::<i64, _>(n)),
                "BLOB" => serde_json::json!(row.get::<Vec<u8>, _>(n)),
                kind => panic!("uncovered SQLite archive type {kind}"),
            }
        };
        values.insert(col.name().into(), value);
    }
    values.into()
}
pub(super) async fn contract(a: &AccountRepository) {
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query(
                "INSERT INTO sync_cursors VALUES('legacy-folder','opaque-cursor','initial')",
            )
            .execute(db)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO provider_refs VALUES('legacy-provider','shared-message','INBOX',11,7)",
            )
            .execute(db)
            .await
            .unwrap();
            sqlx::query("INSERT INTO push_subscriptions VALUES('legacy-push','opaque receipt')")
                .execute(db)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO recipients VALUES('legacy@example.invalid','Éva','deliberate',17)",
            )
            .execute(db)
            .await
            .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query(
                "INSERT INTO sync_cursors VALUES($1,'legacy-folder','opaque-cursor','initial')",
            )
            .bind(&a.account)
            .execute(db)
            .await
            .unwrap();
            sqlx::query("INSERT INTO provider_refs VALUES($1,'legacy-provider','shared-message','INBOX',11,7)").bind(&a.account).execute(db).await.unwrap();
            sqlx::query("INSERT INTO push_subscriptions VALUES($1,'legacy-push','opaque receipt')")
                .bind(&a.account)
                .execute(db)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO recipients VALUES($1,'legacy@example.invalid','Éva','deliberate',17)",
            )
            .bind(&a.account)
            .execute(db)
            .await
            .unwrap();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("account.sqlite");
    a.snapshot_sqlite(&path).await.unwrap();
    assert!(matches!(
        a.snapshot_sqlite(&path).await,
        Err(Error::Invalid)
    ));
    let archive = crate::pool(&path).await.unwrap();
    assert_archive(a, &archive).await;
    if let Backend::Postgres(source) = &a.backend {
        // Restore every populated durable domain, including worker queue order,
        // into a fresh account scope with its original identity still owned.
        sqlx::query("DELETE FROM account_locks WHERE account_id=$1")
            .bind(&a.account)
            .execute(source)
            .await
            .unwrap();
        a.initialize_account().await.unwrap();
        a.import_sqlite(&path).await.unwrap();
        assert_archive(a, &archive).await;
        assert!(matches!(a.import_sqlite(&path).await, Err(Error::Conflict)));
        assert_archive(a, &archive).await;
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM search_documents")
        .fetch_one(&archive)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(!sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM provider_documents WHERE key='search:database')"
    )
    .fetch_one(&archive)
    .await
    .unwrap());
    let capability = AuthorizedAccount {
        id: a.account.clone(),
        user_id: a.owner.clone(),
        identity: Arc::new(()),
    };
    let restored = AccountRepository::sqlite(archive, &capability);
    assert_eq!(
        a.draft("shared-draft").await.unwrap(),
        restored.draft("shared-draft").await.unwrap()
    );
    assert_eq!(
        a.events(0).await.unwrap(),
        restored.events(0).await.unwrap()
    );
    assert_eq!(
        serde_json::to_value(a.sender_cleanup_jobs().await.unwrap()).unwrap(),
        serde_json::to_value(restored.sender_cleanup_jobs().await.unwrap()).unwrap()
    );
    assert_eq!(
        a.removal_busy().await.unwrap(),
        restored.removal_busy().await.unwrap()
    );
    assert_eq!(
        a.provider_document("same-key").await.unwrap(),
        restored.provider_document("same-key").await.unwrap()
    );
    let before = restored.event_cursor().await.unwrap();
    let draft = restored.draft("shared-draft").await.unwrap();
    let revision = draft.revision;
    let edited = restored.save_draft(draft).await.unwrap();
    assert_eq!(edited.revision, revision + 1);
    assert_eq!(restored.event_cursor().await.unwrap(), before + 1);
}

async fn assert_archive(a: &AccountRepository, archive: &SqlitePool) {
    if let Backend::Postgres(source) = &a.backend {
        let tables:Vec<String>=sqlx::query_scalar("SELECT table_name FROM information_schema.tables WHERE table_schema='dakia_mail' AND table_type='BASE TABLE' AND table_name NOT IN ('_sqlx_migrations','account_locks') AND table_name NOT LIKE 'search_%' ORDER BY table_name").fetch_all(source).await.unwrap();
        for table in tables {
            let cols:Vec<String>=sqlx::query_scalar("SELECT column_name FROM information_schema.columns WHERE table_schema='dakia_mail' AND table_name=$1 AND is_generated='NEVER' AND column_name<>'account_id' ORDER BY ordinal_position").bind(&table).fetch_all(source).await.unwrap();
            let native: Vec<_> = cols
                .iter()
                .map(|col| {
                    if ((table == "rule_executions" || table == "push_jobs") && col == "id")
                        || (table == "model_jobs" && col == "queue_id")
                    {
                        format!("{col} AS rowid")
                    } else {
                        col.clone()
                    }
                })
                .collect();
            let local: Vec<_> = native
                .iter()
                .map(|col| col.split(" AS ").last().unwrap())
                .collect();
            let native_query = format!(
                "SELECT {} FROM {table} WHERE account_id=$1{}",
                native.join(","),
                if table == "provider_documents" {
                    " AND key<>'search:database'"
                } else {
                    ""
                }
            );
            let mut expected: Vec<_> = sqlx::query(&native_query)
                .bind(&a.account)
                .fetch_all(source)
                .await
                .unwrap()
                .iter()
                .map(|row| pg_row(row).to_string())
                .collect();
            let mut actual: Vec<_> =
                sqlx::query(&format!("SELECT {} FROM {table}", local.join(",")))
                    .fetch_all(archive)
                    .await
                    .unwrap()
                    .iter()
                    .map(|row| sqlite_row(row).to_string())
                    .collect();
            expected.sort();
            actual.sort();
            assert_eq!(actual, expected, "portable account table {table}");
        }
        let (native_cursor, native_revision): (i64, i64) = sqlx::query_as(
            "SELECT event_cursor,thread_revision FROM account_locks WHERE account_id=$1",
        )
        .bind(&a.account)
        .fetch_one(source)
        .await
        .unwrap();
        let cursor: i64 = sqlx::query_scalar("SELECT seq FROM sqlite_sequence WHERE name='events'")
            .fetch_one(archive)
            .await
            .unwrap();
        assert_eq!(cursor, native_cursor);
        let revision: i64 = sqlx::query_scalar("SELECT revision FROM thread_generation")
            .fetch_one(archive)
            .await
            .unwrap();
        assert_eq!(revision, native_revision);
    }
}
