//! Derived search persistence. No provider calls, credentials, or authorization decisions.
use crate::{normalize_search, CachedSearchFilter, Error, Result};
use dakia_domain::Message;
#[cfg(test)]
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row, SqlitePool};
use std::{path::Path, str::FromStr};

macro_rules! query {
    ($local:expr,$tx:ident,$sql:expr,[$($v:expr),*],$pg:expr,[$($p:expr),*],$method:ident)=>{
        if $local {sqlx::query_scalar($sql)$(.bind($v))*.$method(&mut *$tx).await?}
        else {sqlx::query_scalar($pg)$(.bind($p))*.$method(&mut *$tx).await?}
    };
}
macro_rules! rows {
    ($local:expr,$tx:ident,$sql:expr,[$($v:expr),*],$pg:expr,[$($p:expr),*],$method:ident)=>{
        if $local {sqlx::query_as($sql)$(.bind($v))*.$method(&mut *$tx).await?}
        else {sqlx::query_as($pg)$(.bind($p))*.$method(&mut *$tx).await?}
    };
}
macro_rules! write {
    ($local:expr,$tx:ident,$sql:expr,[$($v:expr),*],$pg:expr,[$($p:expr),*])=>{
        if $local {sqlx::query($sql)$(.bind($v))*.execute(&mut *$tx).await?.rows_affected()}
        else {sqlx::query($pg)$(.bind($p))*.execute(&mut *$tx).await?.rows_affected()}
    };
}
macro_rules! transaction {
    ($self:expr,$account:expr,$tx:ident,$local:ident,$body:block) => {
        match $self {
            SearchDatabase::Sqlite(db) => {
                let mut $tx = db.begin_with("BEGIN IMMEDIATE").await?;
                let $local = true;
                $body
            }
            SearchDatabase::Postgres(db) => {
                let mut $tx = db.begin().await?;
                let $local = false;
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,19217639535198210))")
                    .bind($account)
                    .execute(&mut *$tx)
                    .await?;
                $body
            }
        }
    };
}
const SQLITE_UPSERT: &str = "INSERT INTO documents(account_id,message_id,version,source,metadata,sender,recipients,subject,body,participants,attachment_text,received_at,state,mime_recipients,attachment_details,body_payload,metadata_recipients,received_date,snoozed_until) VALUES(?1,?2,?3,?4,?5,?6,?7||char(10)||?14,?8,?9,?10,?11,?12,?13,?14,?15,?16,?7,?17,?18) ON CONFLICT(account_id,message_id) DO UPDATE SET version=excluded.version,source=excluded.source,metadata=excluded.metadata,sender=excluded.sender,recipients=excluded.metadata_recipients||char(10)||CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.mime_recipients ELSE documents.mime_recipients END,subject=excluded.subject,received_at=excluded.received_at,received_date=excluded.received_date,snoozed_until=excluded.snoozed_until,state=excluded.state,metadata_recipients=excluded.metadata_recipients,body_payload=excluded.body_payload,body=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.body ELSE documents.body END,participants=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.participants ELSE documents.participants END,attachment_text=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.attachment_text ELSE documents.attachment_text END,mime_recipients=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.mime_recipients ELSE documents.mime_recipients END,attachment_details=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.attachment_details ELSE documents.attachment_details END WHERE documents.version<excluded.version";
const SQLITE_METADATA_PATCH: &str = "UPDATE documents SET version=?3,metadata=?5,received_at=?12,received_date=?17,snoozed_until=?18,state=?13,body_payload=0 WHERE account_id=?1 AND message_id=?2 AND source=?4 AND sender=?6 AND metadata_recipients=?7 AND subject=?8 AND version<?3 AND ?16=0";
const POSTGRES_UPSERT: &str = "INSERT INTO documents(account_id,message_id,version,source,metadata,sender,recipients,subject,body,participants,attachment_text,received_at,state,mime_recipients,attachment_details,body_payload,metadata_recipients,received_date,snoozed_until) VALUES($1,$2,$3,$4,$5,$6,$7||chr(10)||$14,$8,$9,$10,$11,$12,$13,$14,$15,$16,$7,$17,$18) ON CONFLICT(account_id,message_id) DO UPDATE SET version=excluded.version,source=excluded.source,metadata=excluded.metadata,sender=excluded.sender,recipients=excluded.metadata_recipients||chr(10)||CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.mime_recipients ELSE documents.mime_recipients END,subject=excluded.subject,received_at=excluded.received_at,received_date=excluded.received_date,snoozed_until=excluded.snoozed_until,state=excluded.state,metadata_recipients=excluded.metadata_recipients,body_payload=excluded.body_payload,body=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.body ELSE documents.body END,participants=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.participants ELSE documents.participants END,attachment_text=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.attachment_text ELSE documents.attachment_text END,mime_recipients=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.mime_recipients ELSE documents.mime_recipients END,attachment_details=CASE WHEN excluded.body_payload=1 OR documents.source<>excluded.source OR excluded.state='deleted' THEN excluded.attachment_details ELSE documents.attachment_details END WHERE documents.version<excluded.version";
const POSTGRES_METADATA_PATCH: &str = "UPDATE documents SET version=$3,metadata=$5,received_at=$12,received_date=$17,snoozed_until=$18,state=$13,body_payload=0 WHERE account_id=$1 AND message_id=$2 AND source=$4 AND sender=$6 AND metadata_recipients=$7 AND subject=$8 AND version<$3 AND $16=0";

#[derive(Clone)]
pub(crate) enum SearchDatabase {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

/// Opaque fault-injection lease. Dropping it releases held control capacity;
/// backend connection types never cross the application repository boundary.
#[cfg(feature = "test-support")]
pub struct SearchTestLease {
    _sqlite: Vec<sqlx::pool::PoolConnection<sqlx::Sqlite>>,
    _postgres: Vec<sqlx::pool::PoolConnection<sqlx::Postgres>>,
}

#[cfg(feature = "test-support")]
impl crate::Store {
    pub async fn hold_search_control_for_test(&self) -> Result<SearchTestLease> {
        let target = self.search_database.as_ref().ok_or(Error::Invalid)?;
        target.warm().await?;
        let mut held = SearchTestLease {
            _sqlite: vec![],
            _postgres: vec![],
        };
        match target.connection().await? {
            SearchDatabase::Sqlite(pool) => {
                for _ in 0..pool.options().get_max_connections() {
                    held._sqlite.push(pool.acquire().await?);
                }
            }
            SearchDatabase::Postgres(pool) => {
                for _ in 0..pool.options().get_max_connections() {
                    held._postgres.push(pool.acquire().await?);
                }
            }
        }
        Ok(held)
    }
}

struct LexicalPage<'a> {
    after: Option<(f64, &'a str, &'a str)>,
    before: Option<(&'a str, &'a str)>,
    ranked: bool,
    limit: i64,
    #[cfg(test)]
    verify_plan: bool,
}
impl Default for LexicalPage<'_> {
    fn default() -> Self {
        Self {
            after: None,
            before: None,
            ranked: false,
            limit: 51,
            #[cfg(test)]
            verify_plan: false,
        }
    }
}

pub(crate) struct Document {
    pub message: Message,
    pub source: String,
    pub sender: String,
    pub recipients: String,
    pub mime_recipients: String,
    pub attachment_details: String,
    pub body_payload: bool,
    pub snoozed_until: Option<String>,
    pub subject: String,
    pub body: String,
    pub participants: String,
    pub attachments: String,
    pub state: String,
}

impl SearchDatabase {
    /// Separate physical pools keep ranking scans from occupying semantic and
    /// control capacity. They share the selected database and its migrations.
    async fn read_lane(&self) -> Result<Self> {
        let capacity = crate::SEARCH_ACCOUNT_CONCURRENCY as u32;
        Ok(match self {
            Self::Sqlite(db) => Self::Sqlite(
                budget::sqlite_pool()
                    .max_connections(capacity)
                    .min_connections(capacity)
                    .connect_with(db.connect_options().as_ref().clone())
                    .await?,
            ),
            Self::Postgres(db) => Self::Postgres(
                crate::database_deadline::postgres_pool_options(db.connect_options().as_ref())
                    .max_connections(capacity)
                    .min_connections(capacity)
                    .acquire_timeout(std::time::Duration::from_secs(3))
                    .connect_with(db.connect_options().as_ref().clone())
                    .await?,
            ),
        })
    }
    pub async fn coverage(&self, account: &str) -> Result<crate::TextCoverage> {
        macro_rules! read {
            ($db:expr,$sql:expr) => {{
                let row = sqlx::query($sql).bind(account).fetch_optional($db).await?;
                row.map(|row| crate::TextCoverage {
                    headers: row.get("headers"),
                    bodies: row.get("bodies"),
                    pending: row.get("pending"),
                    gaps: row.get("gaps"),
                    bytes: row.get("body_bytes"),
                    retrying: row.get("retrying"),
                    ..Default::default()
                })
                .unwrap_or_default()
            }};
        }
        Ok(match self {
            Self::Sqlite(db)=>read!(db,"SELECT headers,bodies,pending,gaps,body_bytes,retrying FROM account_progress WHERE account_id=?"),
            Self::Postgres(db)=>read!(db,"SELECT headers,bodies,pending,gaps,body_bytes,retrying FROM account_progress WHERE account_id=$1"),
        })
    }
    pub async fn open(root: &Path, url: Option<&str>) -> Result<Self> {
        match url {
            Some(url) if url.starts_with("postgres:") || url.starts_with("postgresql:") => {
                let options = sqlx::postgres::PgConnectOptions::from_str(url)?;
                let bootstrap = crate::database_deadline::postgres_pool_options(&options)
                    .max_connections(1)
                    .acquire_timeout(std::time::Duration::from_secs(3))
                    .connect_with(options.clone())
                    .await?;
                // Check before migrations, including the iterative-scan feature
                // needed to retain recall under account/model filtering.
                let extensions:Vec<(String,String)>=sqlx::query_as("SELECT name,COALESCE(installed_version,default_version) FROM pg_available_extensions WHERE name IN ('vector','pg_trgm')").fetch_all(&bootstrap).await?;
                let vector = extensions.iter().find(|(name, _)| name == "vector");
                let supported=vector.is_some_and(|(_,version)|{
                    let mut parts=version.split('.').filter_map(|s|s.parse::<u32>().ok());
                    matches!((parts.next(),parts.next()),(Some(major),Some(minor)) if major>0 || minor>=8)
                });
                if !supported || !extensions.iter().any(|(name, _)| name == "pg_trgm") {
                    bootstrap.close().await;
                    tracing::warn!(
                        operation = "search_database",
                        outcome = "extensions_unavailable",
                        "search requires pgvector 0.8 or later and pg_trgm"
                    );
                    return Err(Error::Invalid);
                }
                sqlx::query("CREATE SCHEMA IF NOT EXISTS dakia_search")
                    .execute(&bootstrap)
                    .await?;
                bootstrap.close().await;
                let db = crate::database_deadline::postgres_pool_options(&options)
                    .max_connections(4)
                    .acquire_timeout(std::time::Duration::from_secs(3))
                    // Search deadlines are shorter than compilation of these ranking expressions.
                    .connect_with(
                        options.options([("search_path", "dakia_search,public"), ("jit", "off")]),
                    )
                    .await?;
                sqlx::migrate!("./migrations/search/postgres")
                    .run(&db)
                    .await?;
                Ok(Self::Postgres(db))
            }
            None => {
                tokio::fs::create_dir_all(root.join("search")).await?;
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(root.join("search").join("index.sqlite"))
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .row_buffer_size(512)
                    .pragma("secure_delete", "ON")
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .busy_timeout(std::time::Duration::from_secs(5));
                let db = budget::sqlite_pool().connect_with(options).await?;
                sqlx::migrate!("./migrations/search/sqlite")
                    .run(&db)
                    .await?;
                Ok(Self::Sqlite(db))
            }
            Some(url) if url.starts_with("sqlite:") => {
                let options = sqlx::sqlite::SqliteConnectOptions::from_str(url)?
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .row_buffer_size(512)
                    .pragma("secure_delete", "ON")
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
                let db = budget::sqlite_pool().connect_with(options).await?;
                sqlx::migrate!("./migrations/search/sqlite")
                    .run(&db)
                    .await?;
                Ok(Self::Sqlite(db))
            }
            _ => Err(Error::Invalid),
        }
    }

    /// Retain tombstone versions so delayed/replayed deliveries cannot resurrect data.
    #[cfg(test)]
    pub async fn deliver(
        &self,
        account: &str,
        id: &str,
        version: i64,
        document: Option<Document>,
    ) -> Result<()> {
        self.deliver_batch(account, &[(id.to_owned(), version, document)])
            .await
    }
    pub async fn dates_known(&self, account: &str) -> Result<bool> {
        let missing: bool = match self {
            Self::Sqlite(db) => sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM documents WHERE account_id=? AND state<>'deleted' AND received_date IS NULL)").bind(account).fetch_one(db).await?,
            Self::Postgres(db) => sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM documents WHERE account_id=$1 AND state<>'deleted' AND received_date IS NULL)").bind(account).fetch_one(db).await?,
        };
        Ok(!missing)
    }
    pub async fn deliver_batch(
        &self,
        account: &str,
        records: &[(String, i64, Option<Document>)],
    ) -> Result<()> {
        macro_rules! write_batch {
            ($db:expr,$sql:expr,$patch:expr,$sqlite:expr,$inline:path) => {{
                let mut tx = if $sqlite {
                    $db.begin_with("BEGIN IMMEDIATE").await?
                } else {
                    $db.begin().await?
                };
                if !$sqlite {
                    sqlx::query(
                        "SELECT pg_advisory_xact_lock(hashtextextended($1,19217639535198210))",
                    )
                    .bind(account)
                    .execute(&mut *tx)
                    .await?;
                }
                for (id, version, document) in records {
                    let metadata = document
                        .as_ref()
                        .map(|d| serde_json::to_string(&d.message))
                        .transpose()?
                        .unwrap_or_else(|| "{}".into());
                    let value =
                        |get: fn(&Document) -> &str| document.as_ref().map(get).unwrap_or("");
                    macro_rules! bind_document {
                        ($query:expr) => {
                            $query
                                .bind(account)
                                .bind(id)
                                .bind(version)
                                .bind(value(|d| &d.source))
                                .bind(&metadata)
                                .bind(value(|d| &d.sender))
                                .bind(value(|d| &d.recipients))
                                .bind(value(|d| &d.subject))
                                .bind(value(|d| &d.body))
                                .bind(value(|d| &d.participants))
                                .bind(value(|d| &d.attachments))
                                .bind(value(|d| &d.message.received_at))
                                .bind(
                                    document
                                        .as_ref()
                                        .map(|d| d.state.as_str())
                                        .unwrap_or("deleted"),
                                )
                                .bind(value(|d| &d.mime_recipients))
                                .bind(
                                    document
                                        .as_ref()
                                        .map(|d| d.attachment_details.as_str())
                                        .unwrap_or("[]"),
                                )
                                .bind(i64::from(document.as_ref().is_some_and(|d| d.body_payload)))
                                .bind(
                                    document
                                        .as_ref()
                                        .and_then(|d| d.message.received_date.as_deref()),
                                )
                                .bind(document.as_ref().and_then(|d| d.snoozed_until.as_deref()))
                        };
                    }
                    if document.as_ref().is_some_and(|d| !d.body_payload)
                        && bind_document!(sqlx::query($patch))
                            .execute(&mut *tx)
                            .await?
                            .rows_affected()
                            == 1
                    {
                        continue;
                    }
                    bind_document!(sqlx::query($sql)).execute(&mut *tx).await?;
                    $inline(&mut tx, account, id).await?;
                }
                tx.commit().await?;
            }};
        }
        match self {
            Self::Sqlite(db) => write_batch!(
                db,
                SQLITE_UPSERT,
                SQLITE_METADATA_PATCH,
                true,
                lexical::inline_sqlite
            ),
            Self::Postgres(db) => write_batch!(
                db,
                POSTGRES_UPSERT,
                POSTGRES_METADATA_PATCH,
                false,
                lexical::inline_postgres
            ),
        }
        Ok(())
    }

    pub async fn identity(&self) -> Result<String> {
        Ok(match self {
            Self::Sqlite(db) => {
                sqlx::query_scalar("SELECT value FROM index_identity WHERE id=1")
                    .fetch_one(db)
                    .await?
            }
            Self::Postgres(db) => {
                sqlx::query_scalar("SELECT value FROM index_identity WHERE id=1")
                    .fetch_one(db)
                    .await?
            }
        })
    }

    pub async fn remove_account(&self, account: &str) -> Result<()> {
        match self {
            Self::Sqlite(db) => {
                let mut tx = db.begin().await?;
                sqlx::query("DELETE FROM chunks WHERE account_id=?")
                    .bind(account)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM documents WHERE account_id=?")
                    .bind(account)
                    .execute(&mut *tx)
                    .await?;
                for table in [
                    "embedding_progress",
                    "embedding_accounts",
                    "lexical_accounts",
                    "account_progress",
                ] {
                    sqlx::query(&format!("DELETE FROM {table} WHERE account_id=?"))
                        .bind(account)
                        .execute(&mut *tx)
                        .await?;
                }
                tx.commit().await?;
            }
            Self::Postgres(db) => {
                let mut tx = db.begin().await?;
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,19217639535198210))")
                    .bind(account)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM chunks WHERE account_id=$1")
                    .bind(account)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM documents WHERE account_id=$1")
                    .bind(account)
                    .execute(&mut *tx)
                    .await?;
                for table in [
                    "embedding_progress",
                    "embedding_accounts",
                    "lexical_accounts",
                    "account_progress",
                ] {
                    sqlx::query(&format!("DELETE FROM {table} WHERE account_id=$1"))
                        .bind(account)
                        .execute(&mut *tx)
                        .await?;
                }
                tx.commit().await?;
            }
        }
        Ok(())
    }

    pub async fn rows(
        &self,
        account: &str,
        filter: &CachedSearchFilter<'_>,
        before: Option<(&str, &str)>,
    ) -> Result<Vec<(Message, String)>> {
        Ok(self
            .lexical_rows(
                account,
                filter,
                LexicalPage {
                    before,
                    ..Default::default()
                },
            )
            .await?
            .into_iter()
            .map(|(m, s, _)| (m, s))
            .collect())
    }
}

/// A search outage must not prevent the mail application from starting. Failed
/// connections are retried by normal worker turns; credentials never enter logs.
pub(crate) struct SearchConnection {
    root: std::path::PathBuf,
    url: Option<String>,
    database: std::sync::Arc<tokio::sync::Mutex<Option<SearchDatabase>>>,
    lexical_read: tokio::sync::OnceCell<SearchDatabase>,
    semantic_read: tokio::sync::OnceCell<SearchDatabase>,
    phrase_read: tokio::sync::OnceCell<SearchDatabase>,
    metadata_read: tokio::sync::OnceCell<SearchDatabase>,
    vectors: crate::search_vectors::VectorAccelerator,
    selectors: selectivity::Selectors,
}
impl SearchConnection {
    pub async fn coverage(&self, account: &str) -> Result<crate::TextCoverage> {
        self.connection().await?.coverage(account).await
    }
    pub fn new(root: &Path, url: Option<&str>) -> Result<Self> {
        if url.is_some_and(|u| {
            !u.starts_with("postgres:")
                && !u.starts_with("postgresql:")
                && !u.starts_with("sqlite:")
        }) {
            return Err(Error::Invalid);
        }
        Ok(Self {
            root: root.to_owned(),
            url: url.map(str::to_owned),
            database: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            lexical_read: tokio::sync::OnceCell::new(),
            semantic_read: tokio::sync::OnceCell::new(),
            phrase_read: tokio::sync::OnceCell::new(),
            metadata_read: tokio::sync::OnceCell::new(),
            vectors: crate::search_vectors::VectorAccelerator::new(root),
            selectors: selectivity::Selectors::default(),
        })
    }
    async fn connection(&self) -> Result<SearchDatabase> {
        let mut guard = self.database.clone().lock_owned().await;
        if let Some(db) = &*guard {
            return Ok(db.clone());
        }
        let (root, url) = (self.root.clone(), self.url.clone());
        // Caller deadlines must neither interrupt migrations nor restart them on every retry.
        tokio::spawn(async move {
            let db = SearchDatabase::open(&root, url.as_deref()).await?;
            *guard = Some(db.clone());
            Ok(db)
        })
        .await
        .map_err(|_| Error::Invalid)?
    }
    async fn lane(&self, lane: &tokio::sync::OnceCell<SearchDatabase>) -> Result<SearchDatabase> {
        Ok(lane
            .get_or_try_init(|| async { self.connection().await?.read_lane().await })
            .await?
            .clone())
    }
    async fn lexical_connection(&self) -> Result<SearchDatabase> {
        self.lane(&self.lexical_read).await
    }
    async fn semantic_connection(&self) -> Result<SearchDatabase> {
        self.lane(&self.semantic_read).await
    }
    async fn phrase_connection(&self) -> Result<SearchDatabase> {
        self.lane(&self.phrase_read).await
    }
    async fn metadata_connection(&self) -> Result<SearchDatabase> {
        self.lane(&self.metadata_read).await
    }
    pub async fn warm(&self) -> Result<()> {
        tokio::try_join!(
            self.lexical_connection(),
            self.semantic_connection(),
            self.phrase_connection(),
            self.metadata_connection()
        )?;
        Ok(())
    }
    pub async fn unload_vectors(&self) -> Result<()> {
        self.vectors.unload().await
    }
    pub async fn maintain_vectors(&self, snapshot: bool) -> Result<bool> {
        let SearchDatabase::Sqlite(db) = self.connection().await? else {
            return Ok(false);
        };
        let worked = self.vectors.advance(&db).await?;
        if snapshot && !worked {
            self.vectors.snapshot(&db).await?;
        }
        Ok(worked)
    }
    pub async fn vectors_ready(&self, account: &str, revision: &str) -> Result<bool> {
        match self.connection().await? {
            SearchDatabase::Sqlite(db) => {
                match self.vectors.caught_up(&db, account, revision).await {
                    Err(Error::Conflict) => Ok(false),
                    result => result,
                }
            }
            SearchDatabase::Postgres(_) => Ok(true),
        }
    }
    pub async fn identity(&self) -> Result<String> {
        self.connection().await?.identity().await
    }
    pub async fn deliver_batch(
        &self,
        account: &str,
        records: &[(String, i64, Option<Document>)],
    ) -> Result<()> {
        self.connection()
            .await?
            .deliver_batch(account, records)
            .await
    }
    pub async fn remove_account(&self, account: &str) -> Result<()> {
        self.connection().await?.remove_account(account).await
    }
    pub async fn rows(
        &self,
        account: &str,
        filter: &CachedSearchFilter<'_>,
        before: Option<(&str, &str)>,
    ) -> Result<Vec<(Message, String)>> {
        let text = filter
            .text_units
            .map(|units| !units.is_empty())
            .unwrap_or_else(|| filter.text.is_some_and(|text| !text.trim().is_empty()));
        if text {
            return self
                .lexical_connection()
                .await?
                .rows(account, filter, before)
                .await;
        }
        self.metadata_connection()
            .await?
            .rows(account, filter, before)
            .await
    }
}

/// The last Store owner aborts startup work. The task holds the connection and
/// control repository, never the Store or this lease, so there is no task cycle.
pub(crate) struct SearchWarm(pub tokio::task::AbortHandle);
impl Drop for SearchWarm {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// The same strict metadata predicates constrain hit aggregation and the
// final ranked head. Alias names come only from these backend implementations.
macro_rules! metadata_predicates {
    ($q:ident,$sqlite:expr,$filter:expr,$alias:expr,$at:expr) => {{
        let criteria = $filter;
        for (column, value) in [
            ("sender", criteria.from),
            ("recipients", criteria.to),
            ("subject", criteria.subject),
        ] {
            if let Some(value) = value {
                let value = normalize_search(value);
                if $sqlite {
                    $q.push(" AND instr(")
                        .push($alias)
                        .push(".")
                        .push(column)
                        .push(",")
                        .push_bind(value)
                        .push(")>0");
                } else {
                    $q.push(" AND ")
                        .push($alias)
                        .push(".")
                        .push(column)
                        .push(" LIKE ")
                        .push_bind(filter::postgres_substring(&value))
                        .push(" ESCAPE '!'");
                }
            }
        }
        for (operator, day) in [(">=", criteria.since), ("<", criteria.before)] {
            if let Some(day) = day {
                $q.push(" AND ")
                    .push($alias)
                    .push(".received_date")
                    .push(operator)
                    .push_bind(day);
            }
        }
        if let Some(value) = criteria.seen {
            $q.push(" AND ")
                .push($alias)
                .push(".unread<>")
                .push_bind(value);
        }
        if criteria.has_attachment {
            $q.push(" AND json_array_length(")
                .push($alias)
                .push(if $sqlite {
                    ".attachment_details)>0"
                } else {
                    ".attachment_details::json)>0"
                });
        }
        if let Some(value) = criteria.starred {
            $q.push(" AND ")
                .push($alias)
                .push(".starred=")
                .push_bind(value);
        }
        if let Some(folders) = criteria.folders {
            $q.push(" AND EXISTS(SELECT 1 FROM document_folders f WHERE f.account_id=")
                .push($alias)
                .push(".account_id AND f.message_id=")
                .push($alias)
                .push(".message_id AND f.folder");
            if $sqlite {
                $q.push(" IN (SELECT value FROM json_each(")
                    .push_bind(serde_json::to_string(folders)?)
                    .push(")))");
            } else {
                $q.push("=ANY(ARRAY(SELECT jsonb_array_elements_text(")
                    .push_bind(serde_json::to_string(folders)?)
                    .push("::jsonb))))");
            }
        }
        crate::classification::predicate!(
            $q,
            $sqlite,
            &format!("{}.metadata", $alias),
            &criteria.classification
        );
        if let Some(category) = criteria.category {
            $q.push(" AND (")
                .push($alias)
                .push(".category IS NULL OR ")
                .push($alias)
                .push(".category=")
                .push_bind(category)
                .push(")");
        }
        if criteria.snoozed_only {
            $q.push(" AND ")
                .push($alias)
                .push(".snoozed_until>")
                .push_bind($at.clone());
        }
        if criteria.hide_snoozed {
            $q.push(" AND (")
                .push($alias)
                .push(".snoozed_until IS NULL OR ")
                .push($alias)
                .push(".snoozed_until<=")
                .push_bind($at.clone())
                .push(")");
        }
    }};
}

// Native positions are exact on SQLite. PostgreSQL truncates very long or
// repetitive position lists, so only those chunks retain canonical verification.
macro_rules! native_field_match {
    ($q:ident,$sqlite:expr,$account:expr,$unit:expr,$field:expr) => {{
        if $sqlite {
            $q.push("lexical_terms MATCH ")
                .push_bind(probe::sqlite_field_match(
                    $account,
                    &[$unit.to_owned()],
                    $field,
                ));
        } else {
            $q.push("(c.lexemes @@ to_tsquery('simple',")
                .push_bind(probe::postgres_field_match($account, $unit, $field))
                .push(")");
            if $unit.split_whitespace().count() > 1 {
                $q.push(" AND ((c.phrase_safe AND c.lexemes @@ to_tsquery('simple',")
                    .push_bind(probe::postgres_phrase($unit, $field))
                    .push(")) OR (NOT c.phrase_safe AND strpos(' '||c.text||' ',")
                    .push_bind(format!(" {} ", $unit))
                    .push(")>0))");
            }
            $q.push(")");
        }
    }};
}

macro_rules! native_match {
    ($q:ident,$sqlite:expr,$account:expr,$unit:expr) => {
        native_field_match!($q, $sqlite, $account, $unit, None)
    };
}

impl SearchDatabase {
    #[cfg(test)]
    pub async fn ranked_rows(
        &self,
        account: &str,
        filter: &CachedSearchFilter<'_>,
        after: Option<(f64, &str, &str)>,
    ) -> Result<Vec<(Message, String, f64)>> {
        self.lexical_rows(
            account,
            filter,
            LexicalPage {
                after,
                ranked: true,
                ..Default::default()
            },
        )
        .await
    }
    async fn lexical_rows(
        &self,
        account: &str,
        filter: &CachedSearchFilter<'_>,
        page: LexicalPage<'_>,
    ) -> Result<Vec<(Message, String, f64)>> {
        let mut timing = crate::timing::SearchTiming::new("lexical_prepare");
        let LexicalPage {
            after,
            before,
            ranked,
            limit,
            #[cfg(test)]
            verify_plan,
        } = page;
        if !(1..=1025).contains(&limit) {
            return Err(Error::Invalid);
        }
        let raw = filter.text_units.map(|u| u.to_vec()).unwrap_or_else(|| {
            filter
                .text
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        });
        let units = raw
            .iter()
            .map(|u| crate::lexical_text::unit(u)?.ok_or(Error::Invalid))
            .collect::<Result<Vec<_>>>()?;
        let alternatives = if let Some(alternatives) = filter.text_alternatives {
            if alternatives.len() != units.len() {
                return Err(Error::Invalid);
            }
            alternatives
                .iter()
                .zip(&units)
                .map(|(variants, original)| {
                    if variants.is_empty() || variants.len() > 9 {
                        return Err(Error::Invalid);
                    }
                    let canonical = variants
                        .iter()
                        .map(|v| crate::lexical_text::unit(v)?.ok_or(Error::Invalid))
                        .collect::<Result<Vec<_>>>()?;
                    if canonical.first() != Some(original) {
                        return Err(Error::Invalid);
                    }
                    Ok(canonical)
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            units.iter().map(|unit| vec![unit.clone()]).collect()
        };
        let filter_at = crate::now();
        let exact = normalize_search(&raw.join(" "));
        let sequence = crate::lexical_text::unit(&raw.join(" "))?;
        let phrase = sequence
            .as_ref()
            .filter(|sequence| sequence.split_whitespace().count() > 1);
        // A single exact phrase already proves the whole-query phrase boost
        // on each hit. Do not rescan the same posting list to establish it again.
        let intrinsic_phrase = phrase.is_some_and(|phrase| {
            units.len() == 1 && alternatives[0].len() == 1 && units[0] == *phrase
        });
        macro_rules! run {
            ($kind:ty,$db:expr,$sqlite:expr,$bounded:path,$snapshot:path)=>{{
                let mut snapshot=None;
                let mut metadata_cohort=None;
                let metadata_anchor=true;
                #[cfg(test)]
                let metadata_anchor=metadata_anchor && (!verify_plan || units.is_empty());
                if $sqlite && metadata_anchor {
                    for (column,index,value) in [
                        ("sender","documents_sender_identity",filter.from),
                        ("recipients","documents_recipients_identity",filter.to),
                        ("subject","documents_subject_identity",filter.subject)
                    ] {
                        let Some(value)=value else {continue;};
                        if snapshot.is_none() {
                            timing.stage("metadata_cohort_pool_wait");
                            snapshot=Some($snapshot($db).await?);
                        }
                        timing.stage("metadata_cohort_probe");
                        let sql=format!("SELECT COALESCE(json_group_array(rowid),'[]') FROM (SELECT rowid FROM documents INDEXED BY {index} WHERE account_id=? AND state<>'deleted' AND instr({column},?)>0 LIMIT 1025)");
                        let ids:String=sqlx::query_scalar::<$kind,String>(&sql).bind(account).bind(normalize_search(value))
                            .fetch_one(snapshot.as_mut().unwrap().connection()).await?;
                        if serde_json::from_str::<Vec<i64>>(&ids)?.len()<1025 {
                            // This contains EVERY canonical literal match. A
                            // larger cohort uses the original unbounded plan.
                            metadata_cohort=Some(ids);
                            break;
                        }
                    }
                }
                if !$sqlite && metadata_anchor && !units.is_empty() && filter::CandidateFilter::from(filter).restricted() {
                    timing.stage("metadata_cohort_pool_wait");
                    snapshot=Some($snapshot($db).await?);
                    timing.stage("metadata_cohort_probe");
                    let mut q=sqlx::QueryBuilder::<$kind>::new("SELECT COALESCE(jsonb_agg(message_id)::text,'[]') FROM (SELECT d.message_id FROM documents d WHERE d.account_id=");
                    q.push_bind(account).push(" AND d.state<>'deleted'");
                    metadata_predicates!(q,false,filter,"d",filter_at);
                    q.push(" LIMIT 4097) eligible");
                    let ids:String=q.build_query_scalar().fetch_one(snapshot.as_mut().unwrap().connection()).await?;
                    if serde_json::from_str::<Vec<String>>(&ids)?.len()<4097 {
                        // Only a complete cohort may replace native discovery; larger scopes retain the full ranker.
                        metadata_cohort=Some(ids);
                    }
                }
                if metadata_cohort.is_none() { snapshot=None; }
                // A complete narrow metadata cohort is also a safe anchor
                // for text ranking. It is never a truncated candidate list.
                let anchor = if !units.is_empty() && metadata_cohort.as_deref()!=Some("[]") {
                    probe::anchor(self, account, &alternatives).await?
                } else {
                    None
                };
                // Prefer a rarer native term over a larger metadata cohort.
                // Discovery still reads its complete posting list, with every
                // strict filter applied by the original ranker.
                if anchor.is_some() {metadata_cohort=None;snapshot=None;}
                let planning =
                    ranked && after.is_none() && before.is_none() && !units.is_empty() && anchor.is_none() && metadata_cohort.is_none();
                #[cfg(test)]
                let planning = planning && !verify_plan;
                if planning {
                    timing.stage("lexical_bound_head");
                    match bound_head::ranked_head(
                        self,
                        account,
                        &alternatives,
                        &exact,
                        phrase.map(String::as_str),
                        limit,
                        Some((filter, &filter_at)),
                    )
                    .await
                    {
                        Ok(Some(rows)) => {
                            return rows
                                .into_iter()
                                .map(|(m, s, score)| Ok((serde_json::from_str(&m)?, s, score)))
                                .collect();
                        }
                        Ok(None) => {}
                        Err(_) => {
                            tracing::debug!(target:"dakia_server::search_timing",stage="lexical_bound_head",outcome="general_fallback","search stage")
                        }
                    }
                }
                // PostgreSQL identity lookups stay correlated even before fresh-account statistics settle.
                let mut q=sqlx::QueryBuilder::<$kind>::new("");
                if $sqlite { q.push("SELECT COALESCE(json_group_array(json_array(metadata,source,score)),'[]') FROM ("); }
                if units.is_empty() {
                if let Some(cohort)=&metadata_cohort {
                    q.push("WITH metadata_cohort AS MATERIALIZED (SELECT CAST(value AS BIGINT) AS rowid FROM json_each(")
                        .push_bind(cohort).push(")) ");
                }
                }
                if !units.is_empty(){
                    q.push("WITH ");
                    if let Some(cohort)=&metadata_cohort {
                        if $sqlite {
                            q.push("anchor AS MATERIALIZED (SELECT d.account_id,d.message_id FROM json_each(")
                                .push_bind(cohort).push(") h CROSS JOIN documents d ON d.rowid=CAST(h.value AS BIGINT) WHERE d.account_id=")
                                .push_bind(account).push(" AND d.state<>'deleted'), ");
                        } else {
                            q.push("anchor AS MATERIALIZED (SELECT ").push_bind(account)
                                .push("::text AS account_id,h.value AS message_id FROM jsonb_array_elements_text(")
                                .push_bind(cohort).push("::jsonb) AS h(value)), ");
                        }
                    } else if let Some(anchor) = anchor {
                        // The probe bounds only estimation. Actual discovery is
                        // unbounded and retains every matching message identity.
                        q.push("anchor AS MATERIALIZED (SELECT DISTINCT c.account_id,c.message_id FROM ");
                        if $sqlite {
                            q.push("lexical_terms CROSS JOIN lexical_chunks c ON c.id=lexical_terms.rowid WHERE lexical_terms MATCH ")
                                .push_bind(probe::sqlite_match(account, &alternatives[anchor]));
                        } else {
                            q.push("lexical_chunks c WHERE (");
                            for (i, variant) in alternatives[anchor].iter().enumerate() {
                                if i>0 { q.push(" OR "); }
                                q.push("c.lexemes @@ to_tsquery('simple',").push_bind(probe::postgres_match(account, variant)).push(")");
                            }
                            q.push(")");
                        }
                        q.push(" AND c.account_id=").push_bind(account).push("), ");
                    }
                    if anchor.is_none() && metadata_cohort.is_none() && !intrinsic_phrase {
                        if let Some(phrase)=phrase {
                            q.push("phrase_hits AS MATERIALIZED (SELECT c.id FROM ");
                            if $sqlite {
                                q.push("lexical_terms CROSS JOIN lexical_chunks c INDEXED BY lexical_chunk_lookup ON c.account_id=").push_bind(account).push(" AND c.id=lexical_terms.rowid WHERE ");
                            } else { q.push("lexical_chunks c WHERE "); }
                            native_match!(q,$sqlite,account,phrase);
                            q.push(" AND c.account_id=").push_bind(account).push("), ");
                        }
                    }
                    q.push("matches AS (");
                    if anchor.is_some() || metadata_cohort.is_some() {

                    for (position,unit) in units.iter().enumerate(){
                        if position>0{q.push(" UNION ALL ");}
                        q.push("SELECT c.account_id,c.message_id,").push(position.to_string()).push(" AS unit,MAX(CASE c.field");
                        for (field,weight) in crate::lexical_text::WEIGHTS.iter().enumerate(){q.push(" WHEN ").push(field.to_string()).push(" THEN ").push(weight.to_string());}
                        q.push(" ELSE 0 END");
                        if alternatives[position].len()>1 {
                            q.push("-CASE WHEN ").push(if $sqlite {"instr("}else{"strpos("})
                                .push("' '||c.text||' ',").push_bind(format!(" {unit} ")).push(")>0 THEN 0 ELSE 1 END");
                        }
                        q.push(") AS weight,");
                        if let Some(phrase)=phrase.filter(|_|position==0) {
                            q.push("MAX(CASE WHEN ").push(if $sqlite {"instr("}else{"strpos("})
                                .push("' '||c.text||' ',").push_bind(format!(" {phrase} ")).push(")>0 THEN CASE c.field");
                            for (field,weight) in crate::lexical_text::PHRASE_WEIGHTS.iter().enumerate(){q.push(" WHEN ").push(field.to_string()).push(" THEN ").push(weight.to_string());}
                            q.push(" ELSE 0 END ELSE 0 END)");
                        } else { q.push("0"); }
                        q.push(" AS phrase_boost,MAX(CASE WHEN c.field=2 THEN 1 ELSE 0 END) AS subject_hit,MAX(CASE WHEN c.field IN (0,1,4) THEN 1 ELSE 0 END) AS participant_hit FROM ");
                        q.push(if $sqlite { "anchor h CROSS JOIN lexical_chunks c ON c.account_id=h.account_id AND c.message_id=h.message_id CROSS JOIN documents current INDEXED BY documents_rank_filter ON current.account_id=c.account_id AND current.message_id=c.message_id CROSS JOIN lexical_accounts a ON a.account_id=c.account_id WHERE 1=1" } else { "anchor h JOIN LATERAL (SELECT * FROM lexical_chunks WHERE account_id=h.account_id AND message_id=h.message_id OFFSET 0) c ON TRUE JOIN LATERAL (SELECT * FROM documents WHERE account_id=c.account_id AND message_id=c.message_id OFFSET 0) current ON TRUE JOIN lexical_accounts a ON a.account_id=c.account_id WHERE 1=1" });
                        q.push(" AND c.account_id=").push_bind(account).push(" AND c.content_version=current.content_version AND c.revision IN (4,5) AND a.revision=").push_bind(crate::lexical_text::REVISION).push(" AND current.state<>'deleted' AND ");
                        q.push("(");
                        for (i,variant) in alternatives[position].iter().enumerate() {
                            if i>0 {q.push(" OR ");}
                            q.push(if $sqlite {"instr(' '||c.text||' ',"}else{"strpos(' '||c.text||' ',"})
                                .push_bind(format!(" {variant} ")).push(")>0");
                        }
                        q.push(")");
                        metadata_predicates!(q,$sqlite,filter,"current",filter_at);
                        q.push(" GROUP BY c.account_id,c.message_id" );
                    }
                    } else {
                        // Native ASCII tokens exactly encode canonical keys. Each
                        // variant can contribute independently, then MAX retains
                        // the shared original-versus-typo and field weighting.
                        q.push("SELECT account_id,message_id,unit,MAX(weight) AS weight,MAX(phrase_boost) AS phrase_boost,MAX(subject_hit) AS subject_hit,MAX(participant_hit) AS participant_hit FROM (");
                        let mut first = true;
                        for (position, variants) in alternatives.iter().enumerate() {
                            for (variant_index, variant) in variants.iter().enumerate() {
                                if !first { q.push(" UNION ALL "); } first=false;
                                q.push("SELECT c.account_id,c.message_id,").push(position.to_string()).push(" AS unit,CASE c.field");
                                for (field, weight) in crate::lexical_text::WEIGHTS.iter().enumerate() { q.push(" WHEN ").push(field.to_string()).push(" THEN ").push((weight-i64::from(variant_index>0)).to_string()); }
                                q.push(" ELSE 0 END AS weight,");
                                if phrase.is_some() && position==0 {
                                    q.push(if intrinsic_phrase { "CASE c.field" } else { "CASE WHEN ph.id IS NOT NULL THEN CASE c.field" });
                                    for (field, weight) in crate::lexical_text::PHRASE_WEIGHTS.iter().enumerate() { q.push(" WHEN ").push(field.to_string()).push(" THEN ").push(weight.to_string()); }
                                    q.push(if intrinsic_phrase { " ELSE 0 END" } else { " ELSE 0 END ELSE 0 END" });
                                } else { q.push("0"); }
                                q.push(" AS phrase_boost,CASE WHEN c.field=2 THEN 1 ELSE 0 END AS subject_hit,CASE WHEN c.field IN (0,1,4) THEN 1 ELSE 0 END AS participant_hit FROM ");
                                if $sqlite {
                                    q.push("lexical_terms CROSS JOIN lexical_chunks c INDEXED BY lexical_chunk_lookup ON c.account_id=").push_bind(account).push(" AND c.id=lexical_terms.rowid CROSS JOIN documents current INDEXED BY documents_rank_filter ON current.account_id=c.account_id AND current.message_id=c.message_id CROSS JOIN lexical_accounts a ON a.account_id=c.account_id");
                                } else {
                                    q.push("lexical_chunks c JOIN LATERAL (SELECT * FROM documents WHERE account_id=c.account_id AND message_id=c.message_id OFFSET 0) current ON TRUE JOIN lexical_accounts a ON a.account_id=c.account_id");
                                }
                                if phrase.is_some() && position==0 && !intrinsic_phrase { q.push(" LEFT JOIN phrase_hits ph ON ph.id=c.id"); }
                                q.push(" WHERE ");
                                native_match!(q,$sqlite,account,variant);
                                q.push(" AND c.account_id=").push_bind(account).push(" AND c.content_version=current.content_version AND c.revision IN (4,5) AND a.revision=").push_bind(crate::lexical_text::REVISION).push(" AND current.state<>'deleted'");
                                metadata_predicates!(q,$sqlite,filter,"current",filter_at);
                            }
                        }
                        q.push(") hits GROUP BY account_id,message_id,unit");
                    }
                    if units.len()==1 {
                        q.push("), evidence AS (SELECT account_id,message_id,weight+phrase_boost AS weight,subject_hit,participant_hit FROM matches");
                    } else {
                        q.push("), evidence AS (SELECT account_id,message_id,SUM(weight)+MAX(phrase_boost) AS weight,MAX(subject_hit) AS subject_hit,MAX(participant_hit) AS participant_hit FROM matches GROUP BY account_id,message_id HAVING count(*)=").push_bind(units.len() as i64);
                    }
                    q.push("), scores AS (SELECT e.account_id,e.message_id,-CAST(e.weight+CASE WHEN e.subject_hit<>0 AND d.subject=").push_bind(&exact)
                        .push(" THEN ").push(crate::lexical_text::EXACT_SUBJECT.to_string()).push(" ELSE 0 END+CASE WHEN e.participant_hit<>0 AND (");
                    // Raw normalized identity boundaries distinguish complete names or
                    // addresses from a word in the middle of a different address.
                    for (column_index,column) in ["sender","recipients","participants"].iter().enumerate() {
                        for (boundary_index,needle) in [format!("\n{exact}\n"),format!("\n{exact} "),format!(" {exact}\n")].iter().enumerate() {
                            if column_index>0 || boundary_index>0 {q.push(" OR ");}
                            q.push(if $sqlite {"instr("}else{"strpos("})
                                .push(if $sqlite {"char(10)||trim(d."}else{"chr(10)||trim(d."})
                                .push(*column).push(if $sqlite {")||char(10),"}else{")||chr(10),"})
                                .push_bind(needle.clone()).push(")>0");
                        }
                    }
                    q.push(") THEN ").push(crate::lexical_text::EXACT_PARTICIPANT.to_string()).push(" ELSE 0 END AS ")
                        .push(if $sqlite {"REAL"}else{"DOUBLE PRECISION"})
                        .push(if $sqlite {") AS score FROM evidence e LEFT JOIN documents d INDEXED BY documents_lexical_score ON (e.subject_hit<>0 OR e.participant_hit<>0) AND d.account_id=e.account_id AND d.message_id=e.message_id)"} else {") AS score FROM evidence e JOIN LATERAL (SELECT * FROM documents WHERE account_id=e.account_id AND message_id=e.message_id OFFSET 0) d ON TRUE)"});
                    // Score the matched identities before fetching their metadata.
                    // SQLite otherwise may start both joins with all account rows.
                    q.push(if $sqlite {", ranked AS MATERIALIZED (SELECT d.account_id,d.message_id,d.received_at,d.source,s.score FROM scores s CROSS JOIN documents d INDEXED BY documents_rank_filter ON d.account_id=s.account_id AND d.message_id=s.message_id WHERE d.account_id="} else {", ranked AS MATERIALIZED (SELECT d.account_id,d.message_id,d.received_at,d.source,s.score FROM scores s JOIN LATERAL (SELECT * FROM documents WHERE account_id=s.account_id AND message_id=s.message_id OFFSET 0) d ON TRUE WHERE d.account_id="}).push_bind(account).push(" AND d.state<>'deleted'");
                }else{
                    q.push("SELECT d.metadata,d.source,CAST(0 AS ").push(if $sqlite{"REAL"}else{"DOUBLE PRECISION"});
                    q.push(if metadata_cohort.is_some() {
                        ") AS score FROM metadata_cohort h CROSS JOIN documents d ON d.rowid=h.rowid WHERE d.account_id="
                    } else if $sqlite {
                        ") AS score FROM documents d INDEXED BY documents_filter_order WHERE d.account_id="
                    } else {
                        ") AS score FROM documents d WHERE d.account_id="
                    }).push_bind(account).push(" AND d.state<>'deleted'");
                }
                metadata_predicates!(q,$sqlite,filter,"d",filter_at);

                if let Some((previous,date,id))=after {
                    let score=if units.is_empty(){"0"}else{"s.score"};
                    q.push(" AND (").push(score).push(">").push_bind(previous).push(" OR (").push(score).push("=").push_bind(previous).push(" AND (d.received_at,").push(if $sqlite {"d.message_id"}else{"d.message_id COLLATE \"C\""}).push(")<(").push_bind(date).push(",").push_bind(id).push(")))");
                }
                if let Some((date,id))=before{q.push(" AND (d.received_at,").push(if $sqlite {"d.message_id"}else{"d.message_id COLLATE \"C\""}).push(")<(").push_bind(date).push(",").push_bind(id).push(")");}
                q.push(if ranked{" ORDER BY score,d.received_at DESC,"}else{" ORDER BY d.received_at DESC,"});
                q.push(if $sqlite {"d.message_id"}else{"d.message_id COLLATE \"C\""}).push(" DESC LIMIT ");
                q.push_bind(limit);
                if !units.is_empty() {
                    // Materialization fences the payload lookup after strict
                    // filters, ordering and the bounded result head. Ranking
                    // must not fetch JSON for every broad lexical candidate.
                    q.push(if $sqlite { ") SELECT d.metadata,r.source,r.score FROM ranked r CROSS JOIN documents d ON d.account_id=r.account_id AND d.message_id=r.message_id ORDER BY " } else { ") SELECT d.metadata,r.source,r.score FROM ranked r JOIN LATERAL (SELECT * FROM documents WHERE account_id=r.account_id AND message_id=r.message_id OFFSET 0) d ON TRUE ORDER BY " });
                    q.push(if ranked {"r.score,r.received_at DESC,"}else{"r.received_at DESC,"});
                    q.push(if $sqlite {"r.message_id"}else{"r.message_id COLLATE \"C\""}).push(" DESC");
                }
                if $sqlite { q.push(")"); }
                #[cfg(test)]
                if $sqlite && verify_plan {
                    // Explain this exact generated statement. NULL placeholders
                    // suffice for planning; only the normal query executes data.
                    let sql = format!("EXPLAIN QUERY PLAN {}", q.sql());
                    let mut check = sqlx::query_as::<$kind, (i64,i64,i64,String)>(&sql);
                    for _ in 0..q.sql().matches('?').count() { check = check.bind(None::<String>); }
                    let plan = check.fetch_all($db).await?;
                    if units.is_empty() {
                        if metadata_cohort.is_some() {
                            // Only a proven complete cohort of <=1,024 rows
                            // may sort; table payloads use integer identities.
                            assert!(plan.iter().any(|row|row.3.contains("SEARCH d USING INTEGER PRIMARY KEY (rowid=?)")),"{plan:?}");
                            assert!(!plan.iter().any(|row|row.3.starts_with("SCAN d")),"{plan:?}");
                        } else {
                            assert!(plan.iter().any(|row|row.3.contains("documents_filter_order (account_id=?)")),"{plan:?}");
                            assert!(!plan.iter().any(|row|row.3.contains("TEMP B-TREE")),"{plan:?}");
                        }
                    } else {
                        assert!(plan.iter().any(|row|row.3=="MATERIALIZE ranked"),"{plan:?}");
                        assert!(plan.iter().any(|row|row.3.contains("documents_rank_filter (account_id=? AND message_id=?")),"{plan:?}");
                        let lookups: Vec<_> = plan.iter().filter(|row| row.3.starts_with("SEARCH d ")).collect();
                        assert!(lookups.len() >= 2 && lookups.iter().all(|row| row.3.contains("message_id=?")), "{plan:?}");
                        assert!(!plan.iter().any(|row| row.3.starts_with("SCAN d")), "{plan:?}");
                    }
                }
                timing.stage("lexical_pool_wait");
                let mut read=if snapshot.is_none() {Some($bounded($db).await?)} else {None};
                let connection=if let Some(snapshot)=snapshot.as_mut() {snapshot.connection()} else {read.as_mut().unwrap().connection()};
                timing.stage("lexical_sql");
                // The SQL limit bounds this batch; all filtering and ordering still happen before it.
                let rows:Vec<(String,String,f64)>=if $sqlite {
                    let query=q.build_query_as::<(String,)>();
                    #[cfg(feature="test-support")]
                    let query=crate::timing::capture_lexical(query)?;
                    let (batch,)=query.fetch_one(connection).await?;
                    serde_json::from_str(&batch)?
                } else {
                    let query=q.build_query_as::<(String,String,f64)>();
                    #[cfg(feature="test-support")]
                    let query=crate::timing::capture_lexical(query)?;
                    query.fetch_all(connection).await?
                };
                timing.stage("lexical_decode");
                rows
            }};
        }
        let rows = match self {
            Self::Sqlite(db) => run!(
                sqlx::Sqlite,
                db,
                true,
                budget::sqlite,
                budget::sqlite_snapshot
            ),
            Self::Postgres(db) => run!(
                sqlx::Postgres,
                db,
                false,
                budget::postgres,
                budget::postgres_snapshot
            ),
        };
        rows.into_iter()
            .map(|(m, s, score)| Ok((serde_json::from_str(&m)?, s, score)))
            .collect()
    }
}

#[cfg(test)]
mod lexical_query_plan_tests {
    use super::*;
    #[tokio::test]
    async fn metadata_filters_keep_date_order_with_only_bounded_cohort_sorting() {
        let dir = tempfile::tempdir().unwrap();
        let target = SearchDatabase::open(dir.path(), None).await.unwrap();
        target
            .lexical_rows(
                "account",
                &CachedSearchFilter {
                    to: Some("common@example.invalid"),
                    ..Default::default()
                },
                LexicalPage {
                    verify_plan: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn lexical_ranking_fetches_metadata_from_matched_identities() {
        let dir = tempfile::tempdir().unwrap();
        let target = SearchDatabase::open(dir.path(), None).await.unwrap();
        target
            .lexical_rows(
                "account",
                &CachedSearchFilter {
                    text: Some("needle"),
                    ..Default::default()
                },
                LexicalPage {
                    ranked: true,
                    verify_plan: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
}

#[path = "search_bound_head.rs"]
mod bound_head;
#[path = "search_completion.rs"]
pub(crate) mod completion;
#[path = "search_embedding.rs"]
pub(crate) mod embedding;
#[path = "search_filter.rs"]
pub(crate) mod filter;
#[path = "search_filter_exact.rs"]
mod filter_exact;
#[path = "search_phrase_constraints.rs"]
mod phrase_constraints;
#[path = "search_candidate_probe.rs"]
mod probe;
#[path = "search_filter_selectivity.rs"]
mod selectivity;
#[path = "search_semantic.rs"]
pub(crate) mod semantic;

#[path = "search_spelling.rs"]
pub(crate) mod spelling;

#[path = "search_lexical.rs"]
pub(crate) mod lexical;

impl SearchConnection {
    pub async fn dates_known(&self, account: &str) -> Result<bool> {
        self.connection().await?.dates_known(account).await
    }
    pub async fn ranked_candidates(
        &self,
        account: &str,
        filter: &CachedSearchFilter<'_>,
        after: Option<(f64, &str, &str)>,
        limit: i64,
    ) -> Result<Vec<(Message, String, f64)>> {
        self.lexical_connection()
            .await?
            .lexical_rows(
                account,
                filter,
                LexicalPage {
                    after,
                    ranked: true,
                    limit,
                    ..Default::default()
                },
            )
            .await
    }
    pub async fn phrase_ids(
        &self,
        account: &str,
        ids: &[String],
        phrases: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        if ids.len() > 200 {
            return Err(Error::Invalid);
        }
        let ids = serde_json::to_string(ids)?;
        let database = self.phrase_connection().await?;
        phrase_constraints::ids(&database, account, &ids, phrases).await
    }
}

#[path = "search_read_budget.rs"]
mod budget;

#[cfg(test)]
#[path = "search_read_lanes_tests.rs"]
mod read_lane_tests;

#[cfg(test)]
#[path = "search_field_upgrade_tests.rs"]
mod field_upgrade_tests;

#[cfg(test)]
mod initialization_deadline_tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_cold_open_keeps_unscoped_migration_initialization() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("search")).unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(dir.path().join("search/index.sqlite"))
            .create_if_missing(true);
        let locker = sqlx::SqlitePool::connect_with(options).await.unwrap();
        sqlx::query("CREATE TABLE migration_fixture(value INTEGER)")
            .execute(&locker)
            .await
            .unwrap();
        let lock = locker.begin_with("BEGIN EXCLUSIVE").await.unwrap();
        let search = SearchConnection::new(dir.path(), None).unwrap();
        let result =
            crate::storage_deadline(std::time::Duration::from_millis(25), search.warm()).await;
        assert!(matches!(result, Err(Error::DeadlineExceeded)));
        assert!(
            search.database.try_lock().is_err(),
            "initialization remains owned after caller cancellation"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        lock.rollback().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), search.warm())
            .await
            .unwrap()
            .unwrap();
        assert!(search.database.lock().await.is_some());
        assert!(uuid::Uuid::parse_str(&search.identity().await.unwrap()).is_ok());
        locker.close().await;
    }
}
