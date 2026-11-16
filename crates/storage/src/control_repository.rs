//! Typed control persistence. SQL and transactions stay behind this boundary;
//! callers cannot select a pool, construct SQL, or obtain a driver-specific row.
use crate::{Result, Store};
use sqlx::{postgres::PgPoolOptions, PgPool, Row, SqlitePool};
use std::str::FromStr;

#[derive(Clone)]
pub struct ControlRepository {
    backend: Backend,
}
#[derive(Clone)]
enum Backend {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

pub struct AuthCounts {
    pub users: i64,
    pub logins: i64,
}
pub struct LoginRecord {
    pub user_id: String,
    pub password_hash: String,
}
pub struct SessionRecord {
    pub user_id: String,
    pub csrf: String,
}
pub struct OidcFlow {
    pub nonce: String,
    pub verifier: String,
}
pub struct MailOAuthFlow {
    pub state_hash: String,
    pub session_hash: String,
    pub user_id: String,
    pub verifier: String,
    pub email: String,
    pub display_name: String,
    pub account_id: Option<String>,
    pub expires_at: i64,
}
pub struct InvitationRecord {
    pub id: String,
    pub expires_at: i64,
}
#[derive(Debug, PartialEq, Eq)]
pub enum OwnerClaim {
    Created,
    AlreadyConfigured,
}
#[derive(Debug, PartialEq, Eq)]
pub enum InvitationClaim {
    Created,
    UsernameTaken,
    InvalidInvitation,
}

// These macros contain explicit native statements, not a SQL dialect translator.
macro_rules! optional {
    ($self:expr,$sqlite:expr,$postgres:expr,[$($bind:expr),*],$row:ident => $body:expr) => {
        match &$self.backend {
            Backend::Sqlite(db)=>sqlx::query($sqlite)$(.bind($bind))*.fetch_optional(db).await?.map(|$row|$body),
            Backend::Postgres(db)=>sqlx::query($postgres)$(.bind($bind))*.fetch_optional(db).await?.map(|$row|$body),
        }
    };
}
macro_rules! execute {
    ($self:expr,$sqlite:expr,$postgres:expr,[$($bind:expr),*]) => {
        match &$self.backend {
            Backend::Sqlite(db)=>sqlx::query($sqlite)$(.bind($bind))*.execute(db).await?.rows_affected(),
            Backend::Postgres(db)=>sqlx::query($postgres)$(.bind($bind))*.execute(db).await?.rows_affected(),
        }
    };
}
macro_rules! transaction {
    ($self:expr,$tx:ident,$sqlite:ident,$body:block) => {
        match &$self.backend {
            Backend::Sqlite(db) => {
                let mut $tx = db.begin().await?;
                let $sqlite = true;
                $body
            }
            Backend::Postgres(db) => {
                let mut $tx = db.begin().await?;
                let $sqlite = false;
                $body
            }
        }
    };
}
macro_rules! statement {
    ($sqlite:expr,$local:expr,$remote:expr) => {
        if $sqlite {
            $local
        } else {
            $remote
        }
    };
}

impl Store {
    pub fn control_repository(&self) -> ControlRepository {
        self.control.clone()
    }
}
impl ControlRepository {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn sqlite_for_test(&self) -> &SqlitePool {
        match &self.backend {
            Backend::Sqlite(db) => db,
            Backend::Postgres(_) => panic!("SQLite-only test fixture"),
        }
    }
    pub(crate) fn sqlite(pool: SqlitePool) -> Self {
        Self {
            backend: Backend::Sqlite(pool),
        }
    }

    pub async fn open_postgres(url: &str) -> Result<Self> {
        let options = sqlx::postgres::PgConnectOptions::from_str(url)?;
        let bootstrap = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await?;
        sqlx::query("CREATE SCHEMA IF NOT EXISTS dakia_control")
            .execute(&bootstrap)
            .await?;
        bootstrap.close().await;
        let db = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", "dakia_control,public")]))
            .await?;
        sqlx::migrate!("./migrations/control_postgres")
            .run(&db)
            .await?;
        Ok(Self {
            backend: Backend::Postgres(db),
        })
    }
    pub async fn user(&self, issuer: &str, subject: &str) -> Result<String> {
        let candidate = uuid::Uuid::new_v4().to_string();
        transaction!(self, tx, sqlite, {
            sqlx::query("UPDATE control_locks SET revision=revision+1 WHERE name='auth'")
                .execute(&mut *tx)
                .await?;
            sqlx::query(statement!(sqlite,"INSERT INTO users(id,issuer,subject) VALUES(?,?,?) ON CONFLICT(issuer,subject) DO NOTHING","INSERT INTO users(id,issuer,subject) VALUES($1,$2,$3) ON CONFLICT(issuer,subject) DO NOTHING")).bind(&candidate).bind(issuer).bind(subject).execute(&mut *tx).await?;
            let user: String = sqlx::query_scalar(statement!(
                sqlite,
                "SELECT id FROM users WHERE issuer=? AND subject=?",
                "SELECT id FROM users WHERE issuer=$1 AND subject=$2"
            ))
            .bind(issuer)
            .bind(subject)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(user)
        })
    }
    pub async fn auth_counts(&self) -> Result<AuthCounts> {
        Ok(optional!(self,"SELECT (SELECT count(*) FROM users) users,(SELECT count(*) FROM local_login) logins","SELECT (SELECT count(*) FROM users) users,(SELECT count(*) FROM local_login) logins",[],r=>AuthCounts {users:r.get("users"),logins:r.get("logins")}).expect("aggregate row"))
    }
    pub async fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(
            optional!(self,"SELECT value FROM settings WHERE key=?","SELECT value FROM settings WHERE key=$1",[key],r=>r.get("value")),
        )
    }
    pub async fn put_setting(&self, key: &str, value: &str) -> Result<()> {
        execute!(self,"INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value","INSERT INTO settings(key,value) VALUES($1,$2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[key,value]);
        Ok(())
    }
    pub async fn login(&self, username: &str) -> Result<Option<LoginRecord>> {
        Ok(
            optional!(self,"SELECT user_id,password_hash FROM local_login WHERE username=?","SELECT user_id,password_hash FROM local_login WHERE username=$1",[username],r=>LoginRecord {user_id:r.get("user_id"),password_hash:r.get("password_hash")}),
        )
    }
    pub async fn is_owner(&self, user: &str) -> Result<bool> {
        Ok(optional!(self,"SELECT EXISTS(SELECT 1 FROM local_login WHERE user_id=? AND is_owner=1) owned","SELECT EXISTS(SELECT 1 FROM local_login WHERE user_id=$1 AND is_owner=1) owned",[user],r=>r.get("owned")).expect("exists row"))
    }
    pub async fn record_login_attempt(&self, bucket: &str, now: i64) -> Result<i64> {
        execute!(
            self,
            "DELETE FROM auth_attempts WHERE window_start<?",
            "DELETE FROM auth_attempts WHERE window_start<$1",
            [now - 120]
        );
        let local="INSERT INTO auth_attempts(bucket,window_start,attempts) VALUES(?,?,1) ON CONFLICT(bucket) DO UPDATE SET attempts=CASE WHEN auth_attempts.window_start<=excluded.window_start-60 THEN 1 ELSE auth_attempts.attempts+1 END,window_start=CASE WHEN auth_attempts.window_start<=excluded.window_start-60 THEN excluded.window_start ELSE auth_attempts.window_start END RETURNING attempts";
        let postgres="INSERT INTO auth_attempts(bucket,window_start,attempts) VALUES($1,$2,1) ON CONFLICT(bucket) DO UPDATE SET attempts=CASE WHEN auth_attempts.window_start<=excluded.window_start-60 THEN 1 ELSE auth_attempts.attempts+1 END,window_start=CASE WHEN auth_attempts.window_start<=excluded.window_start-60 THEN excluded.window_start ELSE auth_attempts.window_start END RETURNING attempts";
        Ok(optional!(self,local,postgres,[bucket,now],r=>r.get("attempts")).expect("upsert row"))
    }
    pub async fn claim_owner(
        &self,
        user: &str,
        username: &str,
        password_hash: &str,
    ) -> Result<OwnerClaim> {
        transaction!(self, tx, sqlite, {
            sqlx::query("UPDATE control_locks SET revision=revision+1 WHERE name='auth'")
                .execute(&mut *tx)
                .await?;
            let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users)")
                .fetch_one(&mut *tx)
                .await?;
            if occupied {
                return Ok(OwnerClaim::AlreadyConfigured);
            }
            sqlx::query(statement!(
                sqlite,
                "INSERT INTO users(id,issuer,subject) VALUES(?,'dakia:local',?)",
                "INSERT INTO users(id,issuer,subject) VALUES($1,'dakia:local',$2)"
            ))
            .bind(user)
            .bind(username)
            .execute(&mut *tx)
            .await?;
            sqlx::query(statement!(sqlite,"INSERT INTO local_login(user_id,username,password_hash,is_owner) VALUES(?,?,?,1)","INSERT INTO local_login(user_id,username,password_hash,is_owner) VALUES($1,$2,$3,1)")).bind(user).bind(username).bind(password_hash).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM settings WHERE key='local_setup_token'")
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(OwnerClaim::Created)
        })
    }
    pub async fn create_session(
        &self,
        token_hash: &str,
        user: &str,
        csrf: &str,
        expires_at: i64,
    ) -> Result<()> {
        execute!(
            self,
            "INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)",
            "INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES($1,$2,$3,$4)",
            [token_hash, user, csrf, expires_at]
        );
        Ok(())
    }
    pub async fn session(&self, token_hash: &str, now: i64) -> Result<Option<SessionRecord>> {
        Ok(
            optional!(self,"SELECT user_id,csrf FROM sessions WHERE token_hash=? AND expires_at>?","SELECT user_id,csrf FROM sessions WHERE token_hash=$1 AND expires_at>$2",[token_hash,now],r=>SessionRecord {user_id:r.get("user_id"),csrf:r.get("csrf")}),
        )
    }
    pub async fn revoke_session(&self, token_hash: &str) -> Result<()> {
        execute!(
            self,
            "DELETE FROM sessions WHERE token_hash=?",
            "DELETE FROM sessions WHERE token_hash=$1",
            [token_hash]
        );
        Ok(())
    }
    pub async fn invitations(&self, owner: &str, now: i64) -> Result<Vec<InvitationRecord>> {
        macro_rules! rows {
            ($db:expr,$sql:expr) => {
                sqlx::query($sql)
                    .bind(owner)
                    .bind(now)
                    .fetch_all($db)
                    .await?
                    .into_iter()
                    .map(|r| InvitationRecord {
                        id: r.get("id"),
                        expires_at: r.get("expires_at"),
                    })
                    .collect()
            };
        }
        Ok(match &self.backend {
            Backend::Sqlite(db)=>rows!(db,"SELECT id,expires_at FROM local_invitations WHERE created_by=? AND expires_at>? ORDER BY expires_at,id"),
            Backend::Postgres(db)=>rows!(db,"SELECT id,expires_at FROM local_invitations WHERE created_by=$1 AND expires_at>$2 ORDER BY expires_at,id"),
        })
    }
    /// The limit is shared across owners, matching the existing workspace policy.
    pub async fn create_invitation(
        &self,
        id: &str,
        hash: &str,
        owner: &str,
        now: i64,
        expires: i64,
    ) -> Result<bool> {
        transaction!(self, tx, sqlite, {
            sqlx::query("UPDATE control_locks SET revision=revision+1 WHERE name='auth'")
                .execute(&mut *tx)
                .await?;
            sqlx::query(statement!(
                sqlite,
                "DELETE FROM local_invitations WHERE expires_at<=?",
                "DELETE FROM local_invitations WHERE expires_at<=$1"
            ))
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let owner_allowed: bool = sqlx::query_scalar(statement!(
                sqlite,
                "SELECT EXISTS(SELECT 1 FROM local_login WHERE user_id=? AND is_owner=1)",
                "SELECT EXISTS(SELECT 1 FROM local_login WHERE user_id=$1 AND is_owner=1)"
            ))
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
            if !owner_allowed {
                return Err(crate::Error::NotFound);
            }
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM local_invitations")
                .fetch_one(&mut *tx)
                .await?;
            if count >= 20 {
                return Ok(false);
            }
            sqlx::query(statement!(sqlite,"INSERT INTO local_invitations(id,token_hash,created_by,expires_at) VALUES(?,?,?,?)","INSERT INTO local_invitations(id,token_hash,created_by,expires_at) VALUES($1,$2,$3,$4)")).bind(id).bind(hash).bind(owner).bind(expires).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(true)
        })
    }
    pub async fn revoke_invitation(&self, id: &str, owner: &str) -> Result<bool> {
        Ok(execute!(
            self,
            "DELETE FROM local_invitations WHERE id=? AND created_by=?",
            "DELETE FROM local_invitations WHERE id=$1 AND created_by=$2",
            [id, owner]
        ) == 1)
    }
    pub async fn invitation_valid(&self, hash: &str, now: i64) -> Result<bool> {
        Ok(optional!(self,"SELECT EXISTS(SELECT 1 FROM local_invitations WHERE token_hash=? AND expires_at>?) valid","SELECT EXISTS(SELECT 1 FROM local_invitations WHERE token_hash=$1 AND expires_at>$2) valid",[hash,now],r=>r.get("valid")).expect("exists row"))
    }
    pub async fn accept_invitation(
        &self,
        hash: &str,
        now: i64,
        user: &str,
        username: &str,
        password_hash: &str,
    ) -> Result<InvitationClaim> {
        transaction!(self, tx, sqlite, {
            sqlx::query("UPDATE control_locks SET revision=revision+1 WHERE name='auth'")
                .execute(&mut *tx)
                .await?;
            let taken: bool = sqlx::query_scalar(statement!(
                sqlite,
                "SELECT EXISTS(SELECT 1 FROM users WHERE issuer='dakia:local' AND subject=?)",
                "SELECT EXISTS(SELECT 1 FROM users WHERE issuer='dakia:local' AND subject=$1)"
            ))
            .bind(username)
            .fetch_one(&mut *tx)
            .await?;
            if taken {
                return Ok(InvitationClaim::UsernameTaken);
            }
            let used = sqlx::query(statement!(
                sqlite,
                "DELETE FROM local_invitations WHERE token_hash=? AND expires_at>?",
                "DELETE FROM local_invitations WHERE token_hash=$1 AND expires_at>$2"
            ))
            .bind(hash)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if used != 1 {
                return Ok(InvitationClaim::InvalidInvitation);
            }
            sqlx::query(statement!(
                sqlite,
                "INSERT INTO users(id,issuer,subject) VALUES(?,'dakia:local',?)",
                "INSERT INTO users(id,issuer,subject) VALUES($1,'dakia:local',$2)"
            ))
            .bind(user)
            .bind(username)
            .execute(&mut *tx)
            .await?;
            sqlx::query(statement!(sqlite,"INSERT INTO local_login(user_id,username,password_hash,is_owner) VALUES(?,?,?,0)","INSERT INTO local_login(user_id,username,password_hash,is_owner) VALUES($1,$2,$3,0)")).bind(user).bind(username).bind(password_hash).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(InvitationClaim::Created)
        })
    }
    pub async fn start_oidc_flow(
        &self,
        hash: &str,
        nonce: &str,
        verifier: &str,
        now: i64,
        expires: i64,
    ) -> Result<()> {
        execute!(
            self,
            "DELETE FROM oidc_flows WHERE expires_at<=?",
            "DELETE FROM oidc_flows WHERE expires_at<=$1",
            [now]
        );
        execute!(
            self,
            "INSERT INTO oidc_flows(state_hash,nonce,verifier,expires_at) VALUES(?,?,?,?)",
            "INSERT INTO oidc_flows(state_hash,nonce,verifier,expires_at) VALUES($1,$2,$3,$4)",
            [hash, nonce, verifier, expires]
        );
        Ok(())
    }
    pub async fn consume_oidc_flow(&self, hash: &str, now: i64) -> Result<Option<OidcFlow>> {
        Ok(
            optional!(self,"DELETE FROM oidc_flows WHERE state_hash=? AND expires_at>? RETURNING nonce,verifier","DELETE FROM oidc_flows WHERE state_hash=$1 AND expires_at>$2 RETURNING nonce,verifier",[hash,now],r=>OidcFlow {nonce:r.get("nonce"),verifier:r.get("verifier")}),
        )
    }
    pub async fn start_gmail_flow(&self, flow: &MailOAuthFlow, now: i64) -> Result<()> {
        transaction!(self, tx, sqlite, {
            let owned:bool=sqlx::query_scalar(statement!(sqlite,"SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND user_id=? AND expires_at>?)","SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=$1 AND user_id=$2 AND expires_at>$3)")).bind(&flow.session_hash).bind(&flow.user_id).bind(now).fetch_one(&mut *tx).await?;
            if !owned {
                return Err(crate::Error::NotFound);
            }
            sqlx::query(statement!(
                sqlite,
                "DELETE FROM gmail_oauth_flows WHERE expires_at<=? OR session_hash=?",
                "DELETE FROM gmail_oauth_flows WHERE expires_at<=$1 OR session_hash=$2"
            ))
            .bind(now)
            .bind(&flow.session_hash)
            .execute(&mut *tx)
            .await?;
            sqlx::query(statement!(sqlite,"INSERT INTO gmail_oauth_flows(state_hash,session_hash,user_id,verifier,email,display_name,account_id,expires_at) VALUES(?,?,?,?,?,?,?,?)","INSERT INTO gmail_oauth_flows(state_hash,session_hash,user_id,verifier,email,display_name,account_id,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")).bind(&flow.state_hash).bind(&flow.session_hash).bind(&flow.user_id).bind(&flow.verifier).bind(&flow.email).bind(&flow.display_name).bind(&flow.account_id).bind(flow.expires_at).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(())
        })
    }
    pub async fn consume_gmail_flow(
        &self,
        hash: &str,
        session: &str,
        user: &str,
        now: i64,
    ) -> Result<Option<MailOAuthFlow>> {
        Ok(
            optional!(self,"DELETE FROM gmail_oauth_flows WHERE state_hash=? AND session_hash=? AND user_id=? AND expires_at>? RETURNING *","DELETE FROM gmail_oauth_flows WHERE state_hash=$1 AND session_hash=$2 AND user_id=$3 AND expires_at>$4 RETURNING *",[hash,session,user,now],r=>MailOAuthFlow {state_hash:r.get("state_hash"),session_hash:r.get("session_hash"),user_id:r.get("user_id"),verifier:r.get("verifier"),email:r.get("email"),display_name:r.get("display_name"),account_id:r.get("account_id"),expires_at:r.get("expires_at")}),
        )
    }
    pub async fn start_microsoft_flow(&self, flow: &MailOAuthFlow, now: i64) -> Result<()> {
        transaction!(self, tx, sqlite, {
            let owned:bool=sqlx::query_scalar(statement!(sqlite,"SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=? AND user_id=? AND expires_at>?)","SELECT EXISTS(SELECT 1 FROM sessions WHERE token_hash=$1 AND user_id=$2 AND expires_at>$3)")).bind(&flow.session_hash).bind(&flow.user_id).bind(now).fetch_one(&mut *tx).await?;
            if !owned {
                return Err(crate::Error::NotFound);
            }
            sqlx::query(statement!(
                sqlite,
                "DELETE FROM microsoft_oauth_flows WHERE expires_at<=? OR session_hash=?",
                "DELETE FROM microsoft_oauth_flows WHERE expires_at<=$1 OR session_hash=$2"
            ))
            .bind(now)
            .bind(&flow.session_hash)
            .execute(&mut *tx)
            .await?;
            sqlx::query(statement!(sqlite,"INSERT INTO microsoft_oauth_flows(state_hash,session_hash,user_id,verifier,email,display_name,account_id,expires_at) VALUES(?,?,?,?,?,?,?,?)","INSERT INTO microsoft_oauth_flows(state_hash,session_hash,user_id,verifier,email,display_name,account_id,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")).bind(&flow.state_hash).bind(&flow.session_hash).bind(&flow.user_id).bind(&flow.verifier).bind(&flow.email).bind(&flow.display_name).bind(&flow.account_id).bind(flow.expires_at).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(())
        })
    }
    pub async fn consume_microsoft_flow(
        &self,
        hash: &str,
        session: &str,
        user: &str,
        now: i64,
    ) -> Result<Option<MailOAuthFlow>> {
        Ok(
            optional!(self,"DELETE FROM microsoft_oauth_flows WHERE state_hash=? AND session_hash=? AND user_id=? AND expires_at>? RETURNING *","DELETE FROM microsoft_oauth_flows WHERE state_hash=$1 AND session_hash=$2 AND user_id=$3 AND expires_at>$4 RETURNING *",[hash,session,user,now],r=>MailOAuthFlow {state_hash:r.get("state_hash"),session_hash:r.get("session_hash"),user_id:r.get("user_id"),verifier:r.get("verifier"),email:r.get("email"),display_name:r.get("display_name"),account_id:r.get("account_id"),expires_at:r.get("expires_at")}),
        )
    }
}

#[cfg(test)]
#[path = "control_repository_tests.rs"]
mod tests;

impl ControlRepository {
    pub(crate) async fn owns_account(&self, user: &str, id: &str) -> Result<bool> {
        Ok(optional!(self,"SELECT EXISTS(SELECT 1 FROM accounts WHERE id=? AND user_id=?) owned","SELECT EXISTS(SELECT 1 FROM accounts WHERE id=$1 AND user_id=$2) owned",[id,user],r=>r.get("owned")).expect("exists row"))
    }
    pub(crate) async fn accounts(&self, user: &str) -> Result<Vec<dakia_domain::Account>> {
        let rows: Vec<String> = match &self.backend {
            Backend::Sqlite(db) => {
                sqlx::query_scalar("SELECT metadata FROM accounts WHERE user_id=? ORDER BY id")
                    .bind(user)
                    .fetch_all(db)
                    .await?
            }
            Backend::Postgres(db) => {
                sqlx::query_scalar("SELECT metadata FROM accounts WHERE user_id=$1 ORDER BY id")
                    .bind(user)
                    .fetch_all(db)
                    .await?
            }
        };
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    pub(crate) async fn all_accounts(&self) -> Result<Vec<(String, dakia_domain::Account)>> {
        macro_rules! rows {
            ($db:expr) => {
                sqlx::query("SELECT user_id,metadata FROM accounts ORDER BY id")
                    .fetch_all($db)
                    .await?
                    .into_iter()
                    .map(|r| (r.get("user_id"), r.get("metadata")))
                    .collect()
            };
        }
        let rows: Vec<(String, String)> = match &self.backend {
            Backend::Sqlite(db) => rows!(db),
            Backend::Postgres(db) => rows!(db),
        };
        rows.into_iter()
            .map(|(user, raw)| Ok((user, serde_json::from_str(&raw)?)))
            .collect()
    }
    pub(crate) async fn account(&self, user: &str, id: &str) -> Result<dakia_domain::Account> {
        let raw: Option<String> = optional!(self,"SELECT metadata FROM accounts WHERE id=? AND user_id=?","SELECT metadata FROM accounts WHERE id=$1 AND user_id=$2",[id,user],r=>r.get("metadata"));
        Ok(serde_json::from_str(&raw.ok_or(crate::Error::NotFound)?)?)
    }
    pub(crate) async fn insert_account(
        &self,
        user: &str,
        account: &dakia_domain::Account,
        credential: &[u8],
        connected_at: &str,
    ) -> Result<()> {
        let metadata = serde_json::to_string(account)?;
        execute!(self,"INSERT INTO accounts(id,user_id,metadata,credential,connected_at) VALUES(?,?,?,?,?)","INSERT INTO accounts(id,user_id,metadata,credential,connected_at) VALUES($1,$2,$3,$4,$5)",[&account.id,user,&metadata,credential,connected_at]);
        Ok(())
    }
    pub(crate) async fn credential(&self, user: &str, id: &str) -> Result<Vec<u8>> {
        optional!(self,"SELECT credential FROM accounts WHERE id=? AND user_id=?","SELECT credential FROM accounts WHERE id=$1 AND user_id=$2",[id,user],r=>r.get("credential")).ok_or(crate::Error::NotFound)
    }
    /// Hold a row write lock before reading its revision on PostgreSQL, and the
    /// existing writer admission on SQLite. The mutation remains shared Rust.
    pub(crate) async fn mutate_account<F>(
        &self,
        user: &str,
        id: &str,
        credential: Option<&[u8]>,
        update: F,
    ) -> Result<dakia_domain::Account>
    where
        F: FnOnce(&mut dakia_domain::Account) -> Result<()> + Send,
    {
        self.mutate_account_credential(user, id, credential, |account, _| update(account))
            .await
    }
    pub(crate) async fn mutate_account_credential<F>(
        &self,
        user: &str,
        id: &str,
        credential: Option<&[u8]>,
        update: F,
    ) -> Result<dakia_domain::Account>
    where
        F: FnOnce(&mut dakia_domain::Account, &[u8]) -> Result<()> + Send,
    {
        transaction!(self, tx, sqlite, {
            let locked = sqlx::query(statement!(
                sqlite,
                "UPDATE accounts SET metadata=metadata WHERE id=? AND user_id=?",
                "UPDATE accounts SET metadata=metadata WHERE id=$1 AND user_id=$2"
            ))
            .bind(id)
            .bind(user)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if locked != 1 {
                return Err(crate::Error::NotFound);
            }
            let raw: String = sqlx::query_scalar(statement!(
                sqlite,
                "SELECT metadata FROM accounts WHERE id=? AND user_id=?",
                "SELECT metadata FROM accounts WHERE id=$1 AND user_id=$2"
            ))
            .bind(id)
            .bind(user)
            .fetch_one(&mut *tx)
            .await?;
            let mut account: dakia_domain::Account = serde_json::from_str(&raw)?;
            let previous: Vec<u8> = sqlx::query_scalar(statement!(
                sqlite,
                "SELECT credential FROM accounts WHERE id=? AND user_id=?",
                "SELECT credential FROM accounts WHERE id=$1 AND user_id=$2"
            ))
            .bind(id)
            .bind(user)
            .fetch_one(&mut *tx)
            .await?;
            update(&mut account, &previous)?;
            sqlx::query(statement!(sqlite,"UPDATE accounts SET metadata=?,credential=COALESCE(?,credential) WHERE id=? AND user_id=?","UPDATE accounts SET metadata=$1,credential=COALESCE($2,credential) WHERE id=$3 AND user_id=$4")).bind(serde_json::to_string(&account)?).bind(credential).bind(id).bind(user).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(account)
        })
    }
    pub(crate) async fn replace_credential(
        &self,
        user: &str,
        id: &str,
        previous: &[u8],
        replacement: &[u8],
    ) -> Result<()> {
        let changed = execute!(
            self,
            "UPDATE accounts SET credential=? WHERE id=? AND user_id=? AND credential=?",
            "UPDATE accounts SET credential=$1 WHERE id=$2 AND user_id=$3 AND credential=$4",
            [replacement, id, user, previous]
        );
        if changed != 1 {
            return Err(crate::Error::Conflict);
        }
        Ok(())
    }
    pub(crate) async fn revoke_account(&self, user: &str, id: &str) -> Result<()> {
        transaction!(self, tx, sqlite, {
            sqlx::query(statement!(sqlite,"INSERT INTO removed_accounts(id) SELECT id FROM accounts WHERE id=? AND user_id=? ON CONFLICT(id) DO NOTHING","INSERT INTO removed_accounts(id) SELECT id FROM accounts WHERE id=$1 AND user_id=$2 ON CONFLICT(id) DO NOTHING")).bind(id).bind(user).execute(&mut *tx).await?;
            let deleted = sqlx::query(statement!(
                sqlite,
                "DELETE FROM accounts WHERE id=? AND user_id=?",
                "DELETE FROM accounts WHERE id=$1 AND user_id=$2"
            ))
            .bind(id)
            .bind(user)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if deleted != 1 {
                return Err(crate::Error::NotFound);
            }
            sqlx::query(statement!(
                sqlite,
                "DELETE FROM search_cursors WHERE user_id=?",
                "DELETE FROM search_cursors WHERE user_id=$1"
            ))
            .bind(user)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn removed_accounts(&self) -> Result<Vec<String>> {
        Ok(match &self.backend {
            Backend::Sqlite(db) => {
                sqlx::query_scalar("SELECT id FROM removed_accounts ORDER BY id")
                    .fetch_all(db)
                    .await?
            }
            Backend::Postgres(db) => {
                sqlx::query_scalar("SELECT id FROM removed_accounts ORDER BY id")
                    .fetch_all(db)
                    .await?
            }
        })
    }
}

impl ControlRepository {
    pub(crate) async fn save_cursor(
        &self,
        user: &str,
        id: &str,
        document: &str,
        now: i64,
        expires: i64,
    ) -> Result<()> {
        transaction!(self, tx, sqlite, {
            sqlx::query(statement!(
                sqlite,
                "DELETE FROM search_cursors WHERE expires_at<=?",
                "DELETE FROM search_cursors WHERE expires_at<=$1"
            ))
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(statement!(
                sqlite,
                "INSERT INTO search_cursors(id,user_id,document,expires_at) VALUES(?,?,?,?)",
                "INSERT INTO search_cursors(id,user_id,document,expires_at) VALUES($1,$2,$3,$4)"
            ))
            .bind(id)
            .bind(user)
            .bind(document)
            .bind(expires)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn cursor(&self, user: &str, id: &str, now: i64) -> Result<Option<String>> {
        Ok(
            optional!(self,"SELECT document FROM search_cursors WHERE id=? AND user_id=? AND expires_at>?","SELECT document FROM search_cursors WHERE id=$1 AND user_id=$2 AND expires_at>$3",[id,user,now],r=>r.get("document")),
        )
    }
    pub(crate) async fn subscribe(&self, user: &str, id: &str, document: &str) -> Result<()> {
        execute!(self,"INSERT INTO push_endpoints(id,user_id,document) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET user_id=excluded.user_id,document=excluded.document","INSERT INTO push_endpoints(id,user_id,document) VALUES($1,$2,$3) ON CONFLICT(id) DO UPDATE SET user_id=excluded.user_id,document=excluded.document",[id,user,document]);
        Ok(())
    }
    pub(crate) async fn push_subscription_enabled(&self, user: &str, id: &str) -> Result<bool> {
        Ok(optional!(self,"SELECT id FROM push_endpoints WHERE id=? AND user_id=?","SELECT id FROM push_endpoints WHERE id=$1 AND user_id=$2",[id,user],r=>r.get::<String,_>("id")).is_some())
    }
    pub(crate) async fn subscriptions(&self, user: &str) -> Result<Vec<serde_json::Value>> {
        let rows: Vec<String> = match &self.backend {
            Backend::Sqlite(db) => {
                sqlx::query_scalar(
                    "SELECT document FROM push_endpoints WHERE user_id=? ORDER BY id",
                )
                .bind(user)
                .fetch_all(db)
                .await?
            }
            Backend::Postgres(db) => {
                sqlx::query_scalar(
                    "SELECT document FROM push_endpoints WHERE user_id=$1 ORDER BY id",
                )
                .bind(user)
                .fetch_all(db)
                .await?
            }
        };
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    pub(crate) async fn expire_subscription(&self, user: &str, id: &str) -> Result<()> {
        execute!(
            self,
            "DELETE FROM push_endpoints WHERE id=? AND user_id=?",
            "DELETE FROM push_endpoints WHERE id=$1 AND user_id=$2",
            [id, user]
        );
        Ok(())
    }
    pub(crate) async fn rules(&self, user: &str) -> Result<Vec<dakia_domain::MailRule>> {
        let rows: Vec<String> = match &self.backend {
            Backend::Sqlite(db) => {
                sqlx::query_scalar(
                    "SELECT document FROM mail_rules WHERE user_id=? ORDER BY sender",
                )
                .bind(user)
                .fetch_all(db)
                .await?
            }
            Backend::Postgres(db) => {
                sqlx::query_scalar(
                    "SELECT document FROM mail_rules WHERE user_id=$1 ORDER BY sender",
                )
                .bind(user)
                .fetch_all(db)
                .await?
            }
        };
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    pub(crate) async fn rule_by_sender(
        &self,
        user: &str,
        email: &str,
    ) -> Result<Option<dakia_domain::MailRule>> {
        let raw: Option<String> = optional!(self,"SELECT document FROM mail_rules WHERE user_id=? AND preset='block_sender' AND sender=?","SELECT document FROM mail_rules WHERE user_id=$1 AND preset='block_sender' AND sender=$2",[user,email],r=>r.get("document"));
        raw.map(|raw| Ok(serde_json::from_str(&raw)?)).transpose()
    }
    pub(crate) async fn rule(&self, user: &str, id: &str) -> Result<dakia_domain::MailRule> {
        let raw: Option<String> = optional!(self,"SELECT document FROM mail_rules WHERE user_id=? AND id=?","SELECT document FROM mail_rules WHERE user_id=$1 AND id=$2",[user,id],r=>r.get("document"));
        Ok(serde_json::from_str(&raw.ok_or(crate::Error::NotFound)?)?)
    }
    pub(crate) async fn upsert_rule(
        &self,
        user: &str,
        email: &str,
        rule: &dakia_domain::MailRule,
    ) -> Result<dakia_domain::MailRule> {
        transaction!(self, tx, sqlite, {
            sqlx::query("UPDATE control_locks SET revision=revision+1 WHERE name='auth'")
                .execute(&mut *tx)
                .await?;
            let previous:Option<String>=sqlx::query_scalar(statement!(sqlite,"SELECT id FROM mail_rules WHERE user_id=? AND preset='block_sender' AND sender=?","SELECT id FROM mail_rules WHERE user_id=$1 AND preset='block_sender' AND sender=$2")).bind(user).bind(email).fetch_optional(&mut *tx).await?;
            let mut canonical = rule.clone();
            if let Some(id) = previous {
                canonical.id = id;
            }
            let document = serde_json::to_string(&canonical)?;
            sqlx::query(statement!(sqlite,"INSERT INTO mail_rules(id,user_id,preset,sender,document) VALUES(?,?,'block_sender',?,?) ON CONFLICT(user_id,preset,sender) DO UPDATE SET document=excluded.document","INSERT INTO mail_rules(id,user_id,preset,sender,document) VALUES($1,$2,'block_sender',$3,$4) ON CONFLICT(user_id,preset,sender) DO UPDATE SET document=excluded.document")).bind(&canonical.id).bind(user).bind(email).bind(document).execute(&mut *tx).await?;
            tx.commit().await?;
            Ok(canonical)
        })
    }
    pub(crate) async fn update_rule(
        &self,
        user: &str,
        rule: &dakia_domain::MailRule,
    ) -> Result<()> {
        let document = serde_json::to_string(rule)?;
        let changed = execute!(
            self,
            "UPDATE mail_rules SET document=? WHERE user_id=? AND id=?",
            "UPDATE mail_rules SET document=$1 WHERE user_id=$2 AND id=$3",
            [document, user, &rule.id]
        );
        if changed != 1 {
            return Err(crate::Error::NotFound);
        }
        Ok(())
    }
    pub(crate) async fn account_connected_at(
        &self,
        user: &str,
        id: &str,
    ) -> Result<Option<String>> {
        let value: Option<Option<String>> = optional!(self,"SELECT connected_at FROM accounts WHERE id=? AND user_id=?","SELECT connected_at FROM accounts WHERE id=$1 AND user_id=$2",[id,user],r=>r.get("connected_at"));
        value.ok_or(crate::Error::NotFound)
    }
}

impl ControlRepository {
    pub(crate) async fn account_owner(&self, id: &str) -> Result<String> {
        optional!(self,"SELECT user_id FROM accounts WHERE id=?","SELECT user_id FROM accounts WHERE id=$1",[id],r=>r.get("user_id")).ok_or(crate::Error::NotFound)
    }
    pub(crate) async fn reset_restored_auth(&self) -> Result<()> {
        transaction!(self, tx, sqlite, {
            for table in ["sessions", "local_invitations", "oidc_flows"] {
                sqlx::query(&format!("DELETE FROM {table}"))
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("INSERT INTO settings(key,value) VALUES('jobs_enabled','false') ON CONFLICT(key) DO UPDATE SET value=excluded.value").execute(&mut *tx).await?;
            let _ = sqlite;
            tx.commit().await?;
            Ok(())
        })
    }
    /// Portable control snapshot. PostgreSQL is copied from one repeatable-read
    /// snapshot into the canonical SQLite archive schema, including ciphertext.
    /// The caller writes its backup manifest only after all stores and blobs finish.
    pub(crate) async fn snapshot_sqlite(&self, path: &std::path::Path) -> Result<()> {
        match &self.backend {
            Backend::Sqlite(db) => {
                sqlx::query("VACUUM INTO ?")
                    .bind(path.to_string_lossy().as_ref())
                    .execute(db)
                    .await?;
            }
            Backend::Postgres(db) => {
                use futures_util::TryStreamExt;
                use sqlx::{Column, TypeInfo, ValueRef};
                const TABLES: [&str; 17] = [
                    "users",
                    "accounts",
                    "sessions",
                    "oidc_flows",
                    "settings",
                    "push_endpoints",
                    "search_cursors",
                    "removed_accounts",
                    "local_login",
                    "login_throttle",
                    "local_invitations",
                    "auth_attempts",
                    "gmail_oauth_flows",
                    "microsoft_oauth_flows",
                    "microsoft_subscriptions",
                    "mail_rules",
                    "control_locks",
                ];
                let mut source = db.begin().await?;
                sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    .execute(&mut *source)
                    .await?;
                let actual:Vec<String>=sqlx::query_scalar("SELECT table_name FROM information_schema.tables WHERE table_schema=current_schema() AND table_type='BASE TABLE' AND table_name<>'_sqlx_migrations' ORDER BY table_name").fetch_all(&mut *source).await?;
                let mut expected = TABLES.map(str::to_owned).to_vec();
                expected.sort();
                if actual != expected {
                    return Err(crate::Error::Invalid);
                }
                if path.exists() {
                    return Err(crate::Error::Invalid);
                }
                let target = crate::pool(path).await?;
                sqlx::migrate!("./migrations/control").run(&target).await?;
                let mut archive = target.begin().await?;
                sqlx::query("DELETE FROM control_locks")
                    .execute(&mut *archive)
                    .await?;
                for table in TABLES {
                    let statement = format!("SELECT * FROM {table}");
                    let mut rows = sqlx::query(&statement).fetch(&mut *source);
                    while let Some(row) = rows.try_next().await? {
                        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
                            "INSERT INTO {table}("
                        ));
                        for (position, column) in row.columns().iter().enumerate() {
                            if position > 0 {
                                query.push(",");
                            }
                            query
                                .push("\"")
                                .push(column.name().replace('"', "\"\""))
                                .push("\"");
                        }
                        query.push(") VALUES(");
                        for (position, column) in row.columns().iter().enumerate() {
                            if position > 0 {
                                query.push(",");
                            }
                            if row.try_get_raw(position)?.is_null() {
                                query.push_bind(Option::<String>::None);
                                continue;
                            }
                            match column.type_info().name() {
                                "TEXT" => {
                                    query.push_bind(row.try_get::<String, _>(position)?);
                                }
                                "INT8" => {
                                    query.push_bind(row.try_get::<i64, _>(position)?);
                                }
                                "BYTEA" => {
                                    query.push_bind(row.try_get::<Vec<u8>, _>(position)?);
                                }
                                _ => return Err(crate::Error::Invalid),
                            }
                        }
                        query.push(")");
                        query.build().execute(&mut *archive).await?;
                    }
                }
                archive.commit().await?;
                source.commit().await?;
                sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                    .execute(&target)
                    .await?;
                target.close().await;
            }
        }
        Ok(())
    }
}
impl ControlRepository {
    pub(crate) async fn import_sqlite(&self, path: &std::path::Path) -> Result<()> {
        let Backend::Postgres(db) = &self.backend else {
            return Err(crate::Error::Invalid);
        };
        let archive = crate::pool(path).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE control_locks SET revision=revision+1 WHERE name='auth'")
            .execute(&mut *tx)
            .await?;
        let populated: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM users) OR EXISTS(SELECT 1 FROM accounts)",
        )
        .fetch_one(&mut *tx)
        .await?;
        if populated {
            return Err(crate::Error::Conflict);
        }
        sqlx::query("DELETE FROM control_locks")
            .execute(&mut *tx)
            .await?;
        for table in [
            "users",
            "accounts",
            "sessions",
            "oidc_flows",
            "settings",
            "push_endpoints",
            "search_cursors",
            "removed_accounts",
            "local_login",
            "login_throttle",
            "local_invitations",
            "auth_attempts",
            "gmail_oauth_flows",
            "microsoft_oauth_flows",
            "microsoft_subscriptions",
            "mail_rules",
            "control_locks",
        ] {
            let columns: Vec<(String,)> = sqlx::query_as(&format!(
                "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
            ))
            .fetch_all(&archive)
            .await?;
            let mapping = columns
                .into_iter()
                .map(|(name,)| name)
                .collect::<Vec<_>>()
                .join(",");
            crate::portable::copy_to_postgres(&archive, &mut tx, table, &mapping, None).await?;
        }
        sqlx::query("SELECT setval(pg_get_serial_sequence('local_login','id'),GREATEST((SELECT last_value FROM local_login_id_seq),COALESCE((SELECT MAX(id) FROM local_login),1)),true)").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO settings(key,value) VALUES('backend_import','incomplete') ON CONFLICT(key) DO UPDATE SET value=excluded.value").execute(&mut *tx).await?;
        tx.commit().await?;
        archive.close().await;
        Ok(())
    }
}
impl ControlRepository {
    pub(crate) async fn close_for_archive(&self) {
        match &self.backend {
            Backend::Sqlite(db) => db.close().await,
            Backend::Postgres(db) => db.close().await,
        }
    }
}

pub struct MicrosoftSubscription {
    pub id: Option<String>,
    pub expires_at: String,
    pub next_attempt: i64,
}
impl ControlRepository {
    pub async fn microsoft_subscription(
        &self,
        account: &str,
    ) -> Result<Option<MicrosoftSubscription>> {
        Ok(
            optional!(self,"SELECT subscription_id,expires_at,next_attempt FROM microsoft_subscriptions WHERE account_id=?","SELECT subscription_id,expires_at,next_attempt FROM microsoft_subscriptions WHERE account_id=$1",[account],r=>MicrosoftSubscription { id:r.get("subscription_id"), expires_at:r.get("expires_at"), next_attempt:r.get("next_attempt") }),
        )
    }
    pub async fn microsoft_wake_version(&self, account: &str) -> Result<i64> {
        Ok(optional!(self,"SELECT wake_version FROM microsoft_subscriptions WHERE account_id=?","SELECT wake_version FROM microsoft_subscriptions WHERE account_id=$1",[account],r=>r.get::<i64,_>("wake_version")).unwrap_or(0))
    }
    pub async fn wake_microsoft_subscription(
        &self,
        renew: bool,
        expires: &str,
        subscription: &str,
        secret: &str,
        now: &str,
    ) -> Result<Option<String>> {
        Ok(
            optional!(self,"UPDATE microsoft_subscriptions SET wake_version=wake_version+1,next_attempt=CASE WHEN ? THEN 0 ELSE next_attempt END,expires_at=CASE WHEN ? THEN ? ELSE expires_at END WHERE subscription_id=? AND client_state_hash=? AND expires_at>? AND EXISTS(SELECT 1 FROM accounts a WHERE a.id=account_id AND json_extract(a.metadata,'$.state')='ready') RETURNING account_id","UPDATE microsoft_subscriptions SET wake_version=wake_version+1,next_attempt=CASE WHEN $1 THEN 0 ELSE next_attempt END,expires_at=CASE WHEN $2 THEN $3 ELSE expires_at END WHERE subscription_id=$4 AND client_state_hash=$5 AND expires_at>$6 AND EXISTS(SELECT 1 FROM accounts a WHERE a.id=account_id AND a.metadata::jsonb->>'state'='ready') RETURNING account_id",[renew,renew,expires,subscription,secret,now],r=>r.get("account_id")),
        )
    }
    pub async fn backoff_microsoft_subscription(
        &self,
        account: &str,
        secret: &str,
        expires: &str,
        next: i64,
    ) -> Result<()> {
        execute!(self,"INSERT INTO microsoft_subscriptions(account_id,client_state_hash,expires_at,next_attempt) VALUES(?,?,?,?) ON CONFLICT(account_id) DO UPDATE SET next_attempt=excluded.next_attempt","INSERT INTO microsoft_subscriptions(account_id,client_state_hash,expires_at,next_attempt) VALUES($1,$2,$3,$4) ON CONFLICT(account_id) DO UPDATE SET next_attempt=excluded.next_attempt",[account,secret,expires,next]);
        Ok(())
    }
    pub async fn save_microsoft_subscription(
        &self,
        account: &str,
        subscription: &str,
        retained: bool,
        secret: &str,
        expires: &str,
    ) -> Result<()> {
        execute!(self,"UPDATE microsoft_subscriptions SET subscription_id=?,client_state_hash=CASE WHEN ? THEN client_state_hash ELSE ? END,expires_at=?,next_attempt=0 WHERE account_id=?","UPDATE microsoft_subscriptions SET subscription_id=$1,client_state_hash=CASE WHEN $2 THEN client_state_hash ELSE $3 END,expires_at=$4,next_attempt=0 WHERE account_id=$5",[subscription,retained,secret,expires,account]);
        Ok(())
    }
    pub async fn expire_microsoft_subscription(&self, account: &str, expires: &str) -> Result<()> {
        execute!(self,"UPDATE microsoft_subscriptions SET subscription_id=NULL,expires_at=?,next_attempt=0 WHERE account_id=?","UPDATE microsoft_subscriptions SET subscription_id=NULL,expires_at=$1,next_attempt=0 WHERE account_id=$2",[expires,account]);
        Ok(())
    }
}
