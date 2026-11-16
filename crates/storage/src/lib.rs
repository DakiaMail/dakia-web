//! Ownership is resolved before opening account databases. All public account access
//! requires an unforgeable capability tied to this Store instance.
mod attachments;
mod backup;
mod cache;
mod classification;
mod graph;
mod inbox;
mod jobs;
mod lists;
mod models;
mod portable;
pub use models::ModelFailureReason;
mod notifications;
mod snoozes;
pub use jobs::PersistedMime;
mod mutation_jobs;
mod rules;
mod sender_cleanup;
pub use sender_cleanup::{CleanupJob, CleanupLocation, CleanupRequest, CleanupTarget};
mod account_repository;
mod content_references;
pub mod control_repository;
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
mod lexical_text;
mod search_backend;
mod search_delivery;
mod search_legacy;
mod search_progress;
mod search_vectors;
pub use full_text::{normalize_search, SearchContent, TextCoverage};
/// Bounded federated account work and matching per-component read capacity.
pub const SEARCH_ACCOUNT_CONCURRENCY: usize = 8;

mod database_deadline;
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
pub use database_deadline::storage_deadline;
use rand::RngCore;
#[cfg(test)]
use sqlx::Row;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
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
    #[error("storage operation deadline exceeded")]
    DeadlineExceeded,
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    /// Keep driver details and stored mail out of diagnostic logs.
    pub fn diagnostic_code(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Sql(sqlx::Error::Database(error)) => {
                error.code().unwrap_or("database_error".into())
            }
            Self::Sql(sqlx::Error::InvalidSavePointStatement) => "invalid_transaction_state".into(),
            Self::Sql(sqlx::Error::PoolTimedOut) => "database_pool_timeout".into(),
            Self::Sql(_) => "database_error".into(),
            Self::Io(_) => "filesystem_error".into(),
            Self::Json(_) => "serialization_error".into(),
            Self::DeadlineExceeded => "deadline_exceeded".into(),
            _ => "storage_error".into(),
        }
    }
}
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
    control: control_repository::ControlRepository,
    key: Arc<[u8; 32]>,
    identity: Arc<()>,
    gate: Arc<RwLock<()>>,
    account_pools: Arc<Mutex<HashMap<String, SqlitePool>>>,
    rules_gate: Arc<RwLock<()>>,
    attachment_budget: u64,
    application_database: Option<account_repository::ApplicationDatabase>,
    default_search_url: Option<Arc<str>>,
    search_database: Option<Arc<search_backend::SearchConnection>>,
    expected_search_model: Option<Arc<str>>,
    search_embeddings_enabled: Arc<std::sync::atomic::AtomicBool>,
    search_warm: Option<Arc<search_backend::SearchWarm>>,
    search_cleanup_done: Arc<std::sync::atomic::AtomicBool>,
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
        // Let a 200-message validation page fit in the bounded row buffer,
        // rather than applying backpressure at the driver's 50-row default.
        .row_buffer_size(512)
        .pragma("secure_delete", "ON")
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    Ok(sqlite_pool_options().connect_with(options).await?)
}
fn sqlite_pool_options() -> SqlitePoolOptions {
    SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| Box::pin(database_deadline::sqlite(connection)))
        .before_acquire(|connection, _| Box::pin(async move {
            database_deadline::sqlite(connection).await?;
            Ok(true)
        }))
        .after_release(|connection, _| {
            Box::pin(async move {
                // Wait for queued rollbacks before checking SQLx's transaction state.
                connection.lock_handle().await?.remove_progress_handler();
                let reusable = !sqlx::Connection::is_in_transaction(connection);
                if !reusable {
                    tracing::warn!(target: "dakia_server::diagnostics", operation = "storage", reason = "invalid_transaction_state", "discarding unusable database connection");
                }
                Ok(reusable)
            })
        })
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
        Self::open_with_database(root, key, attachment_budget, None).await
    }
    /// Select the complete application backend without inspecting process state.
    /// SQLite keeps account files and durable blobs in the existing data root.
    pub async fn open_with_database(
        root: impl AsRef<Path>,
        key: [u8; 32],
        attachment_budget: u64,
        url: Option<&str>,
    ) -> Result<Self> {
        Self::open_database_mode(root, key, attachment_budget, url, false).await
    }
    /// Offline commands retain an exclusive database-wide PostgreSQL lease in
    /// addition to the caller's local data-directory lock.
    pub async fn open_offline_database(
        root: impl AsRef<Path>,
        key: [u8; 32],
        url: Option<&str>,
    ) -> Result<Self> {
        Self::open_database_mode(root, key, 1024 * 1024 * 1024, url, true).await
    }
    async fn open_database_mode(
        root: impl AsRef<Path>,
        key: [u8; 32],
        attachment_budget: u64,
        url: Option<&str>,
        offline: bool,
    ) -> Result<Self> {
        if attachment_budget == 0 || root.as_ref().join("backend-import-in-progress").exists() {
            return Err(Error::Invalid);
        }
        tokio::fs::create_dir_all(root.as_ref().join("accounts")).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [root.as_ref().to_owned(), root.as_ref().join("accounts")] {
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
            }
        }
        let postgres = url.is_some_and(|value| {
            value.starts_with("postgresql://") || value.starts_with("postgres://")
        });
        let lease = if postgres {
            Some(account_repository::DatabaseLease::acquire(url.unwrap(), offline).await?)
        } else {
            None
        };
        let control = if postgres {
            control_repository::ControlRepository::open_postgres(url.unwrap()).await?
        } else {
            let control = match url {
                None => pool(&root.as_ref().join("control.sqlite")).await?,
                Some(value) if value.starts_with("sqlite:") => {
                    let options = SqliteConnectOptions::from_str(value)?
                        .create_if_missing(true)
                        .foreign_keys(true)
                        .pragma("secure_delete", "ON")
                        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                        .busy_timeout(std::time::Duration::from_secs(5));
                    sqlite_pool_options().connect_with(options).await?
                }
                Some(_) => return Err(Error::Invalid),
            };
            sqlx::migrate!("./migrations/control").run(&control).await?;
            control_repository::ControlRepository::sqlite(control)
        };
        if control.setting("backend_import").await?.as_deref() == Some("incomplete") {
            return Err(Error::Invalid);
        }
        let mut store =
            Self::from_control(root.as_ref().to_owned(), key, attachment_budget, control).await?;
        if postgres {
            store.application_database = Some(
                account_repository::ApplicationDatabase::open_with_lease(
                    url.unwrap(),
                    lease.unwrap(),
                )
                .await?,
            );
            store.default_search_url = Some(Arc::from(url.unwrap()));
            for (user, account) in store.control_repository().all_accounts().await? {
                let a = store.authorize(&user, &account.id).await?;
                let repository = store.account_repository(&a).await?;
                repository.initialize_account().await?;
                store
                    .cleanup_received_orphans(&a, repository.cache_blobs().await?)
                    .await?;
            }
            // PostgreSQL has no embedded compatibility FTS. The independently
            // managed search tables default to the same database.
            store.configure_search_database(None).await?;
        }
        Ok(store)
    }
    pub(crate) async fn from_control(
        root: PathBuf,
        key: [u8; 32],
        attachment_budget: u64,
        control: control_repository::ControlRepository,
    ) -> Result<Self> {
        if attachment_budget == 0 {
            return Err(Error::Invalid);
        }
        tokio::fs::create_dir_all(root.join("accounts")).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&root, &root.join("accounts")] {
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
            }
        }
        let store = Self {
            root,
            control,
            key: Arc::new(key),
            identity: Arc::new(()),
            gate: Arc::new(RwLock::new(())),
            account_pools: Arc::new(Mutex::new(HashMap::new())),
            rules_gate: Arc::new(RwLock::new(())),
            attachment_budget,
            application_database: None,
            default_search_url: None,
            search_database: None,
            expected_search_model: None,
            search_embeddings_enabled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            search_warm: None,
            search_cleanup_done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        store.cleanup_removed_accounts().await?;
        Ok(store)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn sqlite_control_for_test(&self) -> &SqlitePool {
        self.control.sqlite_for_test()
    }
    pub async fn user(&self, issuer: &str, subject: &str) -> Result<String> {
        let _guard = self.gate.read().await;
        self.control_repository().user(issuer, subject).await
    }
    pub async fn authorize(&self, user: &str, id: &str) -> Result<AuthorizedAccount> {
        let exists = self.control_repository().owns_account(user, id).await?;
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
        if self.application_database.is_some() {
            return Err(Error::Invalid);
        }
        if !Arc::ptr_eq(&self.identity, &a.identity) {
            return Err(Error::NotFound);
        }
        // Serialize opening with revocation. Cached handles still require current
        // ownership; stale capabilities must not recreate removed databases.
        let mut timing = timing::Timing::new("account_pool_lock");
        let mut pools = self.account_pools.clone().lock_owned().await;
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
        let store = self.clone();
        let a = a.clone();
        // A caller's deadline must not repeatedly roll back a long first-open migration.
        tokio::spawn(async move {
            let db = pool(&store.root.join("accounts").join(format!("{}.sqlite", a.id))).await?;
            search_legacy::prepare(&db, store.search_database.is_some()).await?;
            sqlx::migrate!("./migrations/account")
                .run(&db)
                .await
                .map_err(|error| {
                    tracing::warn!(target: "dakia_server::diagnostics", operation = "account_migration", error = ?error, "account migration failed");
                    error
                })?;
            search_legacy::configure(&db, store.search_database.is_some()).await?;
            let retained = account_repository::AccountRepository::sqlite(db.clone(), &a)
                .cache_blobs()
                .await?;
            store.cleanup_received_orphans(&a, retained).await?;
            pools.insert(a.id.clone(), db.clone());
            Ok(db)
        })
        .await
        .map_err(|_| Error::Invalid)?
    }
    pub async fn accounts(&self, user: &str) -> Result<Vec<Account>> {
        self.control_repository().accounts(user).await
    }
    pub async fn connect(&self, user: &str, input: &ConnectAccount) -> Result<Account> {
        let _guard = self.gate.read().await;
        if input.email.is_empty()
            || (input.password.is_empty()
                && input.gmail_oauth.is_none()
                && input.microsoft_oauth.is_none())
            || (input.provider == ProviderKind::Microsoft && input.microsoft_oauth.is_none())
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
        self.control_repository()
            .insert_account(
                user,
                &a,
                &sealed,
                &Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true),
            )
            .await?;
        let scope = self.authorize(user, &a.id).await?;
        self.account_repository(&scope)
            .await?
            .initialize_account()
            .await?;
        Ok(a)
    }
    /// Replace credentials in-place. Mail identity and durable account data do not change.
    pub async fn reconnect(
        &self,
        a: &AuthorizedAccount,
        input: &ConnectAccount,
    ) -> Result<Account> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        if (input.password.is_empty()
            && input.gmail_oauth.is_none()
            && input.microsoft_oauth.is_none())
            || (input.provider == ProviderKind::Microsoft && input.microsoft_oauth.is_none())
            || input.username.is_empty()
            || input.imap_host.is_empty()
            || input.smtp_host.is_empty()
            || input.imap_port == 0
            || input.smtp_port == 0
        {
            return Err(Error::Invalid);
        }
        let sealed = self.seal_credential(&format!("{}:{}", a.user_id, a.id), input)?;
        self.control_repository()
            .mutate_account_credential(&a.user_id, &a.id, Some(&sealed), |account, previous| {
                let same_identity = if account.provider == ProviderKind::Microsoft
                    && input.provider == ProviderKind::Microsoft
                {
                    self.open_credential(a, previous)?
                        .microsoft_oauth
                        .zip(input.microsoft_oauth.clone())
                        .is_some_and(|(old, new)| {
                            !old.subject.is_empty() && old.subject == new.subject
                        })
                } else {
                    false
                };
                if (!account.email.eq_ignore_ascii_case(&input.email) && !same_identity)
                    || account.provider != input.provider
                {
                    return Err(Error::Invalid);
                }
                if same_identity {
                    account.email = input.email.clone();
                }
                account.state = AccountState::Connecting;
                account.display_name = input.display_name.clone();
                account.revision = account.revision.checked_add(1).ok_or(Error::Invalid)?;
                Ok(())
            })
            .await
    }
    fn seal_credential(&self, aad: &str, input: &ConnectAccount) -> Result<Vec<u8>> {
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| Error::Crypto)?;
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
        Ok(sealed)
    }
    /// Compare-and-swap prevents a late token refresh from overwriting reconnection.
    pub async fn rotate_microsoft_token(
        &self,
        a: &AuthorizedAccount,
        old: &str,
        new: &str,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        let sealed = self
            .control_repository()
            .credential(&a.user_id, &a.id)
            .await?;
        let mut input = self.open_credential(a, &sealed)?;
        let grant = input.microsoft_oauth.as_mut().ok_or(Error::Conflict)?;
        if grant.refresh_token != old || new.is_empty() {
            return Err(Error::Conflict);
        }
        grant.refresh_token = new.into();
        let replacement = self.seal_credential(&format!("{}:{}", a.user_id, a.id), &input)?;
        self.control_repository()
            .replace_credential(&a.user_id, &a.id, &sealed, &replacement)
            .await?;
        Ok(())
    }
    pub async fn credential(&self, a: &AuthorizedAccount) -> Result<ConnectAccount> {
        self.account_repository(a).await?;
        let sealed = self
            .control_repository()
            .credential(&a.user_id, &a.id)
            .await?;
        self.open_credential(a, &sealed)
    }
    fn open_credential(&self, a: &AuthorizedAccount, sealed: &[u8]) -> Result<ConnectAccount> {
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
        self.account_repository(a).await?.draft(id).await
    }
    pub async fn save_draft(&self, a: &AuthorizedAccount, draft: Draft) -> Result<Draft> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?.save_draft(draft).await
    }
    pub async fn submit(
        &self,
        a: &AuthorizedAccount,
        request: &SubmitRequest,
    ) -> Result<Submission> {
        let _guard = self.gate.read().await;
        let due = match &request.due_at {
            Some(s) => DateTime::parse_from_rfc3339(s)
                .map_err(|_| Error::Invalid)?
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true),
            None => now(),
        };
        self.account_repository(a)
            .await?
            .submit(request, &due)
            .await
    }
    pub async fn submission(&self, a: &AuthorizedAccount, id: &str) -> Result<Submission> {
        self.account_repository(a).await?.submission(id).await
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
        self.account_repository(a)
            .await?
            .transition(id, from, to)
            .await
    }
    pub async fn recover_expired(&self, a: &AuthorizedAccount, at: &str) -> Result<u32> {
        let rows = self
            .account_repository(a)
            .await?
            .expired_submissions(at)
            .await?;
        let mut count = 0;
        for (id, state) in rows {
            let (from, to) = if state == "submitting" {
                (
                    SubmissionState::Submitting,
                    SubmissionState::DeliveryUnknown,
                )
            } else {
                (SubmissionState::Claimed, SubmissionState::Scheduled)
            };
            match self.transition(a, &id, from, to).await {
                Ok(_) => count += 1,
                Err(Error::Conflict) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(count)
    }
    pub async fn event_cursor(&self, a: &AuthorizedAccount) -> Result<u32> {
        self.account_repository(a).await?.event_cursor().await
    }
    pub async fn events(&self, a: &AuthorizedAccount, after: u32) -> Result<Vec<Event>> {
        self.account_repository(a).await?.events(after).await
    }
}

pub use search_backend::embedding::{EmbeddedPassage, EmbeddingCoverage, EmbeddingTurn};
pub use search_backend::semantic::SemanticMatch;
#[cfg(feature = "test-support")]
pub use search_backend::SearchTestLease;
pub use search_vectors::check_search_accelerator;
