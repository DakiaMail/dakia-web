//! Ownership is resolved before opening account databases. All public account access
//! requires an unforgeable capability tied to this Store instance.
mod attachments;
mod backup;
mod cache;
mod inbox;
mod jobs;
mod lists;
mod models;
pub use models::ModelFailureReason;
mod notifications;
pub use jobs::PersistedMime;
mod mutation_jobs;
mod sender_cleanup;
pub use sender_cleanup::{CleanupJob, CleanupLocation, CleanupRequest, CleanupTarget};
mod content_references;
mod operations;
#[cfg(test)]
mod pool_tests;
mod provider_state;
pub use content_references::ContentReferences;
mod recipients;
mod remove;
mod search;
pub use search::CachedSearchFilter;
mod full_text;
pub use full_text::{normalize_search, TextCoverage};
mod sender_mail;
mod sync;
mod thread_graph;
mod timing;
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use dakia_domain::*;
use rand::RngCore;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage error")]
    Sql(#[from] sqlx::Error),
    #[error("migration error")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("serialization error")]
    Json(#[from] serde_json::Error),
    #[error("filesystem error")]
    Io(#[from] std::io::Error),
    #[error("resource not found")]
    NotFound,
    #[error("revision or idempotency conflict")]
    Conflict,
    #[error("cleanup target has a pending or unverified move")]
    CleanupMoveUnverified,
    #[error("invalid input")]
    Invalid,
    #[error("credential encryption failed")]
    Crypto,
    #[error("cursor expired; refetch required")]
    CursorExpired,
    #[error("durable attachment quota exceeded")]
    QuotaExceeded,
    #[error("account has pending or uncertain mail actions")]
    AccountBusy,
}
pub type Result<T> = std::result::Result<T, Error>;
/// Held for the process lifetime by the server or offline operator command.
pub fn lock_data_directory(root: impl AsRef<Path>) -> Result<std::fs::File> {
    std::fs::create_dir_all(root.as_ref())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.as_ref().join("operator.lock"))?;
    fs2::FileExt::try_lock_exclusive(&file)?;
    Ok(file)
}
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    control: SqlitePool,
    key: Arc<[u8; 32]>,
    identity: Arc<()>,
    gate: Arc<RwLock<()>>,
    account_pools: Arc<Mutex<HashMap<String, SqlitePool>>>,
    attachment_budget: u64,
}
#[derive(Clone, Debug)]
pub struct AuthorizedAccount {
    id: String,
    user_id: String,
    identity: Arc<()>,
}
impl AuthorizedAccount {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
async fn pool(path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite:")?
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .pragma("secure_delete", "ON")
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}
impl Store {
    pub async fn open(root: impl AsRef<Path>, key: [u8; 32]) -> Result<Self> {
        Self::open_with_attachment_budget(root, key, 1024 * 1024 * 1024).await
    }
    pub async fn open_with_attachment_budget(
        root: impl AsRef<Path>,
        key: [u8; 32],
        attachment_budget: u64,
    ) -> Result<Self> {
        if attachment_budget == 0 {
            return Err(Error::Invalid);
        }
        tokio::fs::create_dir_all(root.as_ref().join("accounts")).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(root.as_ref(), std::fs::Permissions::from_mode(0o700))
                .await?;
            tokio::fs::set_permissions(
                root.as_ref().join("accounts"),
                std::fs::Permissions::from_mode(0o700),
            )
            .await?;
        }
        let control = pool(&root.as_ref().join("control.sqlite")).await?;
        sqlx::migrate!("./migrations/control").run(&control).await?;
        let store = Self {
            root: root.as_ref().to_owned(),
            control,
            key: Arc::new(key),
            identity: Arc::new(()),
            gate: Arc::new(RwLock::new(())),
            account_pools: Arc::new(Mutex::new(HashMap::new())),
            attachment_budget,
        };
        store.cleanup_removed_accounts().await?;
        Ok(store)
    }
    pub fn control(&self) -> &SqlitePool {
        &self.control
    }
    pub async fn user(&self, issuer: &str, subject: &str) -> Result<String> {
        let _guard = self.gate.read().await;
        sqlx::query("INSERT OR IGNORE INTO users(id,issuer,subject) VALUES(?,?,?)")
            .bind(Uuid::new_v4().to_string())
            .bind(issuer)
            .bind(subject)
            .execute(&self.control)
            .await?;
        Ok(
            sqlx::query_scalar("SELECT id FROM users WHERE issuer=? AND subject=?")
                .bind(issuer)
                .bind(subject)
                .fetch_one(&self.control)
                .await?,
        )
    }
    pub async fn authorize(&self, user: &str, id: &str) -> Result<AuthorizedAccount> {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM accounts WHERE id=? AND user_id=?)")
                .bind(id)
                .bind(user)
                .fetch_one(&self.control)
                .await?;
        if !exists {
            return Err(Error::NotFound);
        }
        Ok(AuthorizedAccount {
            id: id.into(),
            user_id: user.into(),
            identity: self.identity.clone(),
        })
    }
    async fn db(&self, a: &AuthorizedAccount) -> Result<SqlitePool> {
        if !Arc::ptr_eq(&self.identity, &a.identity) {
            return Err(Error::NotFound);
        }
        // Serialize opening with revocation. Cached handles still require current
        // ownership; stale capabilities must not recreate removed databases.
        let mut timing = timing::Timing::new("account_pool_lock");
        let mut pools = self.account_pools.lock().await;
        timing.stage("ownership");
        self.authorize(&a.user_id, &a.id).await?;
        timing.stage("account_pool_open");
        // IDs originate exclusively from UUID generation, never client paths.
        if Uuid::parse_str(&a.id).is_err() {
            return Err(Error::NotFound);
        }
        if let Some(db) = pools.get(&a.id) {
            return Ok(db.clone());
        }
        let db = pool(&self.root.join("accounts").join(format!("{}.sqlite", a.id))).await?;
        sqlx::migrate!("./migrations/account").run(&db).await?;
        self.cleanup_received_orphans(a, &db).await?;
        pools.insert(a.id.clone(), db.clone());
        Ok(db)
    }
    pub async fn accounts(&self, user: &str) -> Result<Vec<Account>> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT metadata FROM accounts WHERE user_id=? ORDER BY id")
                .bind(user)
                .fetch_all(&self.control)
                .await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub async fn connect(&self, user: &str, input: &ConnectAccount) -> Result<Account> {
        let _guard = self.gate.read().await;
        if input.email.is_empty()
            || input.password.is_empty()
            || input.imap_host.is_empty()
            || input.smtp_host.is_empty()
            || input.imap_port == 0
            || input.smtp_port == 0
        {
            return Err(Error::Invalid);
        }
        let a = Account {
            id: Uuid::new_v4().to_string(),
            email: input.email.clone(),
            display_name: input.display_name.clone(),
            provider: input.provider.clone(),
            state: AccountState::Connecting,
            revision: 1,
        };
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| Error::Crypto)?;
        let aad = format!("{user}:{}", a.id);
        let encrypted = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &serde_json::to_vec(input)?,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::Crypto)?;
        let mut sealed = nonce.to_vec();
        sealed.extend(encrypted);
        sqlx::query("INSERT INTO accounts(id,user_id,metadata,credential) VALUES(?,?,?,?)")
            .bind(&a.id)
            .bind(user)
            .bind(serde_json::to_string(&a)?)
            .bind(sealed)
            .execute(&self.control)
            .await?;
        let scope = self.authorize(user, &a.id).await?;
        self.db(&scope).await?;
        Ok(a)
    }
    /// Replace credentials in-place. Mail identity and durable account data do not change.
    pub async fn reconnect(
        &self,
        a: &AuthorizedAccount,
        input: &ConnectAccount,
    ) -> Result<Account> {
        let _guard = self.gate.read().await;
        self.db(a).await?;
        if input.password.is_empty()
            || input.username.is_empty()
            || input.imap_host.is_empty()
            || input.smtp_host.is_empty()
            || input.imap_port == 0
            || input.smtp_port == 0
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.control.begin().await?;
        sqlx::query("UPDATE accounts SET metadata=metadata WHERE id=? AND user_id=?")
            .bind(&a.id)
            .bind(&a.user_id)
            .execute(&mut *tx)
            .await?;
        let raw: String =
            sqlx::query_scalar("SELECT metadata FROM accounts WHERE id=? AND user_id=?")
                .bind(&a.id)
                .bind(&a.user_id)
                .fetch_one(&mut *tx)
                .await?;
        let mut account: Account = serde_json::from_str(&raw)?;
        if !account.email.eq_ignore_ascii_case(&input.email) || account.provider != input.provider {
            return Err(Error::Invalid);
        }
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| Error::Crypto)?;
        let aad = format!("{}:{}", a.user_id, a.id);
        let encrypted = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &serde_json::to_vec(input)?,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::Crypto)?;
        let mut sealed = nonce.to_vec();
        sealed.extend(encrypted);
        account.state = AccountState::Connecting;
        account.display_name = input.display_name.clone();
        account.revision = account.revision.checked_add(1).ok_or(Error::Invalid)?;
        sqlx::query("UPDATE accounts SET metadata=?,credential=? WHERE id=? AND user_id=?")
            .bind(serde_json::to_string(&account)?)
            .bind(sealed)
            .bind(&a.id)
            .bind(&a.user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(account)
    }
    pub async fn credential(&self, a: &AuthorizedAccount) -> Result<ConnectAccount> {
        self.db(a).await?;
        let sealed: Vec<u8> =
            sqlx::query_scalar("SELECT credential FROM accounts WHERE id=? AND user_id=?")
                .bind(&a.id)
                .bind(&a.user_id)
                .fetch_one(&self.control)
                .await?;
        if sealed.len() < 28 {
            return Err(Error::Crypto);
        }
        let aad = format!("{}:{}", a.user_id, a.id);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| Error::Crypto)?;
        let nonce: [u8; 12] = sealed[..12].try_into().map_err(|_| Error::Crypto)?;
        let plain = cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &sealed[12..],
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| Error::Crypto)?;
        Ok(serde_json::from_slice(&plain)?)
    }
    pub async fn draft(&self, a: &AuthorizedAccount, id: &str) -> Result<Draft> {
        let db = self.db(a).await?;
        let s: Option<String> = sqlx::query_scalar("SELECT document FROM drafts WHERE id=?")
            .bind(id)
            .fetch_optional(&db)
            .await?;
        Ok(serde_json::from_str(&s.ok_or(Error::NotFound)?)?)
    }
    pub async fn save_draft(&self, a: &AuthorizedAccount, mut draft: Draft) -> Result<Draft> {
        let _guard = self.gate.read().await;
        if draft.account_id != a.id || draft.id.is_empty() || draft.subject.contains(['\r', '\n']) {
            return Err(Error::Invalid);
        }
        if draft
            .attachments
            .iter()
            .map(|x| u64::from(x.size_bytes))
            .sum::<u64>()
            > 20 * 1024 * 1024
        {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        // Obtain the writer lock before reading the revision.
        sqlx::query("UPDATE drafts SET revision=revision WHERE id=?")
            .bind(&draft.id)
            .execute(&mut *tx)
            .await?;
        let current: Option<i64> = sqlx::query_scalar("SELECT revision FROM drafts WHERE id=?")
            .bind(&draft.id)
            .fetch_optional(&mut *tx)
            .await?;
        if current.unwrap_or(0) != i64::from(draft.revision) {
            return Err(Error::Conflict);
        }
        for attachment in &draft.attachments {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM blobs WHERE id=? AND size_bytes=?)",
            )
            .bind(&attachment.id)
            .bind(attachment.size_bytes)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(Error::Invalid);
            }
        }
        draft.revision = draft.revision.checked_add(1).ok_or(Error::Invalid)?;
        draft.saved_at = now();
        sqlx::query("INSERT INTO drafts(id,revision,document) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document=excluded.document").bind(&draft.id).bind(draft.revision).bind(serde_json::to_string(&draft)?).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM draft_blobs WHERE draft_id=?")
            .bind(&draft.id)
            .execute(&mut *tx)
            .await?;
        for attachment in &draft.attachments {
            sqlx::query("INSERT OR IGNORE INTO draft_blobs(draft_id,blob_id) VALUES(?,?)")
                .bind(&draft.id)
                .bind(&attachment.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE blobs SET pinned=1 WHERE id=?")
                .bind(&attachment.id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'draft_saved',?)")
            .bind(draft.revision)
            .bind(&draft.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(draft)
    }
    pub async fn submit(&self, a: &AuthorizedAccount, r: &SubmitRequest) -> Result<Submission> {
        let _guard = self.gate.read().await;
        if r.operation_id.is_empty() || r.timezone.is_empty() {
            return Err(Error::Invalid);
        }
        let due = match &r.due_at {
            Some(s) => DateTime::parse_from_rfc3339(s)
                .map_err(|_| Error::Invalid)?
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            None => now(),
        };
        let request = serde_json::to_string(r)?;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE submissions SET state=state WHERE operation_id=?")
            .bind(&r.operation_id)
            .execute(&mut *tx)
            .await?;
        if let Some(row) =
            sqlx::query("SELECT request,document FROM submissions WHERE operation_id=?")
                .bind(&r.operation_id)
                .fetch_optional(&mut *tx)
                .await?
        {
            if row.get::<String, _>("request") != request {
                return Err(Error::Conflict);
            }
            return Ok(serde_json::from_str(row.get("document"))?);
        }
        let raw: Option<String> =
            sqlx::query_scalar("SELECT document FROM drafts WHERE id=? AND revision=?")
                .bind(&r.draft_id)
                .bind(r.expected_revision)
                .fetch_optional(&mut *tx)
                .await?;
        let raw = raw.ok_or(Error::Conflict)?;
        let d: Draft = serde_json::from_str(&raw)?;
        if d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty() {
            return Err(Error::Invalid);
        }
        let s = Submission {
            id: Uuid::new_v4().to_string(),
            account_id: a.id.clone(),
            operation_id: r.operation_id.clone(),
            draft_id: d.id.clone(),
            draft_revision: d.revision,
            state: SubmissionState::Scheduled,
            due_at: due,
            timezone: r.timezone.clone(),
            filing_status: FilingStatus::Pending,
            revision: 1,
        };
        sqlx::query("INSERT INTO submissions(id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES(?,?,?,?,?,?,?,'scheduled',?,'pending')").bind(&s.id).bind(&s.operation_id).bind(request).bind(&d.id).bind(d.revision).bind(raw).bind(serde_json::to_string(&s)?).bind(&s.due_at).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO outbox(kind,target_id) VALUES('submit',?)")
            .bind(&s.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO events(revision,kind,target_id) VALUES(1,'submission_scheduled',?)",
        )
        .bind(&s.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(s)
    }
    pub async fn submission(&self, a: &AuthorizedAccount, id: &str) -> Result<Submission> {
        let db = self.db(a).await?;
        let raw: Option<String> = sqlx::query_scalar("SELECT document FROM submissions WHERE id=?")
            .bind(id)
            .fetch_optional(&db)
            .await?;
        Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
    }
    /// CAS transition and event are atomic. Never turn an uncertain delivery back into scheduled.
    pub async fn transition(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        from: SubmissionState,
        to: SubmissionState,
    ) -> Result<Submission> {
        let _guard = self.gate.read().await;
        use SubmissionState::*;
        if !matches!(
            (&from, &to),
            (Scheduled, Claimed)
                | (Scheduled, Cancelled)
                | (Claimed, Cancelled)
                | (Claimed, Submitting)
                | (Claimed, Scheduled)
                | (Claimed, Failed)
                | (Submitting, Accepted)
                | (Submitting, Failed)
                | (Submitting, DeliveryUnknown)
        ) {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE submissions SET state=state WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let raw: Option<String> = sqlx::query_scalar("SELECT document FROM submissions WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let mut s: Submission = serde_json::from_str(&raw.ok_or(Error::NotFound)?)?;
        if s.state != from {
            return Err(Error::Conflict);
        }
        s.state = to;
        s.revision += 1;
        let state = serde_json::to_value(&s.state)?
            .as_str()
            .ok_or(Error::Invalid)?
            .to_owned();
        let lease =
            (Utc::now() + Duration::seconds(60)).to_rfc3339_opts(SecondsFormat::Millis, true);
        sqlx::query("UPDATE submissions SET state=?,document=?,lease_until=? WHERE id=?")
            .bind(state)
            .bind(serde_json::to_string(&s)?)
            .bind(lease)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'submission_changed',?)")
            .bind(s.revision)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(s)
    }
    pub async fn recover_expired(&self, a: &AuthorizedAccount, at: &str) -> Result<u32> {
        let db = self.db(a).await?;
        let rows=sqlx::query("SELECT id,state FROM submissions WHERE state IN ('claimed','submitting') AND lease_until <= ?").bind(at).fetch_all(&db).await?;
        let mut count = 0;
        for row in rows {
            let state: String = row.get("state");
            let (from, to) = if state == "submitting" {
                (
                    SubmissionState::Submitting,
                    SubmissionState::DeliveryUnknown,
                )
            } else {
                (SubmissionState::Claimed, SubmissionState::Scheduled)
            };
            match self.transition(a, row.get("id"), from, to).await {
                Ok(_) => count += 1,
                Err(Error::Conflict) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(count)
    }
    pub async fn event_cursor(&self, a: &AuthorizedAccount) -> Result<u32> {
        let db = self.db(a).await?;
        Ok(
            sqlx::query_scalar("SELECT COALESCE(MAX(cursor),0) FROM events")
                .fetch_one(&db)
                .await?,
        )
    }
    pub async fn events(&self, a: &AuthorizedAccount, after: u32) -> Result<Vec<Event>> {
        let db = self.db(a).await?;
        let min: Option<i64> = sqlx::query_scalar("SELECT MIN(cursor) FROM events")
            .fetch_one(&db)
            .await?;
        if after > 0 && min.is_some_and(|n| i64::from(after) + 1 < n) {
            return Err(Error::CursorExpired);
        }
        let rows=sqlx::query("SELECT cursor,revision,kind,target_id FROM events WHERE cursor>? ORDER BY cursor LIMIT 500").bind(after).fetch_all(&db).await?;
        Ok(rows
            .into_iter()
            .map(|r| Event {
                account_id: a.id.clone(),
                cursor: r.get("cursor"),
                revision: r.get("revision"),
                kind: r.get("kind"),
                target_id: r.get("target_id"),
            })
            .collect())
    }
}
