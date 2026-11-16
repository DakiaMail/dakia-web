//! Canonical offline account archives. SQL schemas and engine-specific queue
//! ordering are explicit; generated projections never replace durable records.
use super::*;
use futures_util::TryStreamExt;
use sqlx::{Column, Row, TypeInfo, ValueRef};
struct Table {
    name: &'static str,
    columns: &'static str,
}
// Dependency order matters for foreign keys and graph/header triggers.
const TABLES:&[Table]=&[
    Table{name:"drafts",columns:"id,revision,document"},
    Table{name:"blobs",columns:"id,size_bytes,pinned,last_access"},
    Table{name:"attachment_metadata",columns:"id,document"},
    Table{name:"draft_blobs",columns:"draft_id,blob_id"},
    Table{name:"messages",columns:"id,revision,document,category_override"},
    Table{name:"message_classifications",columns:"message_id,automatic,kind_override,added_types,removed_types,attention_override,requested"},
    Table{name:"classification_senders",columns:"sender,kind"},
    Table{name:"operations",columns:"id,request,document"},
    Table{name:"submissions",columns:"id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,lease_until,filing_status,scheduled_by_user"},
    Table{name:"outbox",columns:"id,kind,target_id,state,lease_until"},
    Table{name:"events",columns:"cursor,revision,kind,target_id"},
    Table{name:"submission_mime",columns:"submission_id,raw,recipients,message_id"},
    Table{name:"provider_documents",columns:"key,document"},
    Table{name:"snoozes",columns:"thread_id,until,arrival_boundary"},
    Table{name:"rule_inbox_observations",columns:"message_id"},
    Table{name:"rule_executions",columns:"id AS rowid,rule_id,message_id,operation_id,state"},
    Table{name:"folder_scans",columns:"folder,generation,continuation,completed_at,last_scanned_at,initial_boundary,uidvalidity,coverage_limited"},
    Table{name:"folder_memberships",columns:"folder,message_id,generation"},
    Table{name:"notification_dedup",columns:"message_id"},
    Table{name:"push_jobs",columns:"id AS rowid,message_id,subscription_id,document,state,attempts,next_attempt"},
    Table{name:"model_jobs",columns:"queue_id AS rowid,id,document,state,retry_count,retry_at,failure_reason"},
    Table{name:"message_identities",columns:"provider_id,message_id"},
    Table{name:"received_cache",columns:"message_id,blob_id,size_bytes,last_access"},
    Table{name:"recipient_evidence",columns:"source_id,email,document,observed_at"},
    Table{name:"thread_edges",columns:"message_id,edge"},
    Table{name:"thread_verifications",columns:"component,revision,checked_at"},
    Table{name:"search_excluded_folders",columns:"name"},
    Table{name:"sync_cursors",columns:"folder_id,cursor,initial_boundary"},
    Table{name:"provider_refs",columns:"provider_key,message_id,folder_id,uidvalidity,uid"},
    Table{name:"push_subscriptions",columns:"id,document"},
    Table{name:"recipients",columns:"email,name,evidence,score"},
];
const DERIVED: &[&str] = &[
    "message_types",
    "account_locks",
    "search_dirty",
    "search_documents",
    "search_index_outbox",
    "search_totals",
    "search_inventory",
    "search_semantic_profile",
    "search_semantic_totals",
    "search_stage_totals",
    "search_semantic_receipts",
    "search_cleanup_work",
    "search_cleanup_totals",
];
impl AccountRepository {
    /// A portable account snapshot, excluding rebuildable extracted text. Native
    /// PostgreSQL rows stream from one repeatable-read snapshot into SQLite.
    pub(crate) async fn snapshot_sqlite(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(Error::Invalid);
        }
        match &self.backend {
            Backend::Sqlite(db) => {
                sqlx::query("VACUUM INTO ?")
                    .bind(path.to_string_lossy().as_ref())
                    .execute(db)
                    .await?;
            }
            Backend::Postgres(db) => {
                let mut source = db.begin().await?;
                sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    .execute(&mut *source)
                    .await?;
                let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM dakia_control.accounts WHERE id=$1 AND user_id=$2)").bind(&self.account).bind(&self.owner).fetch_one(&mut *source).await?;
                if !owned {
                    return Err(Error::NotFound);
                }
                let actual:Vec<String>=sqlx::query_scalar("SELECT table_name FROM information_schema.tables WHERE table_schema=current_schema() AND table_type='BASE TABLE' AND table_name<>'_sqlx_migrations' ORDER BY table_name").fetch_all(&mut *source).await?;
                let mut expected: Vec<_> = TABLES
                    .iter()
                    .map(|table| table.name.to_owned())
                    .chain(DERIVED.iter().map(|name| (*name).to_owned()))
                    .collect();
                expected.sort();
                // A new durable table must be classified before exports can pass.
                if actual != expected {
                    return Err(Error::Invalid);
                }
                let target = crate::pool(path).await?;
                crate::search_legacy::prepare(&target, true).await?;
                sqlx::migrate!("./migrations/account").run(&target).await?;
                crate::search_legacy::configure(&target, true).await?;
                let mut archive = target.begin().await?;
                sqlx::query("DELETE FROM search_excluded_folders")
                    .execute(&mut *archive)
                    .await?;
                for table in TABLES {
                    let filter = if table.name == "provider_documents" {
                        " AND key<>'search:database'"
                    } else {
                        ""
                    };
                    let query = format!(
                        "SELECT {} FROM {} WHERE account_id=$1{filter}",
                        table.columns, table.name
                    );
                    let mut rows = sqlx::query(&query).bind(&self.account).fetch(&mut *source);
                    while let Some(row) = rows.try_next().await? {
                        let mut insert = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
                            "INSERT INTO {}(",
                            table.name
                        ));
                        for (n, column) in row.columns().iter().enumerate() {
                            if n > 0 {
                                insert.push(",");
                            }
                            insert.push("\"").push(column.name()).push("\"");
                        }
                        insert.push(") VALUES(");
                        for (n, column) in row.columns().iter().enumerate() {
                            if n > 0 {
                                insert.push(",");
                            }
                            if row.try_get_raw(n)?.is_null() {
                                insert.push_bind(Option::<String>::None);
                                continue;
                            }
                            match column.type_info().name() {
                                "TEXT" => {
                                    insert.push_bind(row.try_get::<String, _>(n)?);
                                }
                                "INT8" => {
                                    insert.push_bind(row.try_get::<i64, _>(n)?);
                                }
                                "BYTEA" => {
                                    insert.push_bind(row.try_get::<Vec<u8>, _>(n)?);
                                }
                                "BOOL" => {
                                    insert.push_bind(row.try_get::<bool, _>(n)?);
                                }
                                _ => return Err(Error::Invalid),
                            }
                        }
                        insert.push(")");
                        // Header triggers may already have reconstructed an edge.
                        if table.name == "thread_edges" {
                            insert.push(" ON CONFLICT DO NOTHING");
                        }
                        insert.build().execute(&mut *archive).await?;
                    }
                }
                let state: Option<(i64, i64)> = sqlx::query_as(
                    "SELECT event_cursor,thread_revision FROM account_locks WHERE account_id=$1",
                )
                .bind(&self.account)
                .fetch_optional(&mut *source)
                .await?;
                if let Some((cursor, revision)) = state {
                    sqlx::query("DELETE FROM sqlite_sequence WHERE name='events'")
                        .execute(&mut *archive)
                        .await?;
                    sqlx::query("INSERT INTO sqlite_sequence(name,seq) VALUES('events',?)")
                        .bind(cursor)
                        .execute(&mut *archive)
                        .await?;
                    sqlx::query("UPDATE thread_generation SET revision=? WHERE id=1")
                        .bind(revision)
                        .execute(&mut *archive)
                        .await?;
                }
                sqlx::query("UPDATE search_totals SET eligible=(SELECT count(*) FROM search_eligible_messages) WHERE id=1").execute(&mut *archive).await?;
                archive.commit().await?;
                source.commit().await?;
                sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                    .execute(&target)
                    .await?;
                target.close().await;
            }
        }
        // Derived search text is never retained in a portable account archive.
        let archive = crate::pool(path).await?;
        crate::search_legacy::prepare(&archive, true).await?;
        crate::search_legacy::configure(&archive, true).await?;
        sqlx::query("DELETE FROM provider_documents WHERE key='search:database'")
            .execute(&archive)
            .await?;
        sqlx::query("DELETE FROM search_documents")
            .execute(&archive)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO search_dirty SELECT id FROM messages")
            .execute(&archive)
            .await?;
        sqlx::query("VACUUM").execute(&archive).await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&archive)
            .await?;
        archive.close().await;
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn import_sqlite(&self, path: &Path) -> Result<()> {
        let Backend::Postgres(db) = &self.backend else {
            return Err(Error::Invalid);
        };
        let archive = crate::pool(path).await?;
        let mut tx = self.begin_postgres(db, true).await?;
        // A partially populated destination is never merged or overwritten.
        for table in TABLES
            .iter()
            .filter(|table| table.name != "search_excluded_folders")
        {
            let present: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS(SELECT 1 FROM {} WHERE account_id=$1)",
                table.name
            ))
            .bind(&self.account)
            .fetch_one(&mut *tx)
            .await?;
            if present {
                return Err(Error::Conflict);
            }
        }
        sqlx::query("DELETE FROM search_excluded_folders WHERE account_id=$1")
            .bind(&self.account)
            .execute(&mut *tx)
            .await?;
        for table in TABLES {
            crate::portable::copy_to_postgres(
                &archive,
                &mut tx,
                table.name,
                table.columns,
                Some(&self.account),
            )
            .await?;
        }
        let cursor: Option<i64> =
            sqlx::query_scalar("SELECT seq FROM sqlite_sequence WHERE name='events'")
                .fetch_optional(&archive)
                .await?;
        let revision: i64 = sqlx::query_scalar("SELECT revision FROM thread_generation WHERE id=1")
            .fetch_one(&archive)
            .await?;
        sqlx::query(
            "UPDATE account_locks SET event_cursor=$1,thread_revision=$2 WHERE account_id=$3",
        )
        .bind(cursor.unwrap_or(0))
        .bind(revision)
        .bind(&self.account)
        .execute(&mut *tx)
        .await?;
        // Exclusion mappings are imported after messages to retain FK order.
        sqlx::query("UPDATE search_inventory i SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=i.account_id AND e.id=i.message_id) WHERE i.account_id=$1").bind(&self.account).execute(&mut *tx).await?;
        for (table, column) in [
            ("outbox", "id"),
            ("rule_executions", "id"),
            ("push_jobs", "id"),
            ("model_jobs", "queue_id"),
        ] {
            // Identity sequences are global, while archive row IDs are scoped
            // to an account. Never move a sequence behind an existing value.
            sqlx::query(&format!("SELECT setval(pg_get_serial_sequence('{table}','{column}'),GREATEST((SELECT last_value FROM {table}_{column}_seq),COALESCE((SELECT MAX({column}) FROM {table}),1)),true)")).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        archive.close().await;
        Ok(())
    }
}
