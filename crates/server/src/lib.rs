pub mod diagnostics;
mod discovery;
pub mod executor;
pub mod gmail_oauth;
pub mod microsoft_oauth;
mod graph_sync;
mod graph_push;
pub mod idle;
mod images;
mod language;
mod local_auth;
mod local_users;
pub mod mail;
pub mod models;
mod mutations;
pub mod push;
mod releases;
mod rules;
mod search;
mod sender_cleanup;
mod sender_mail;
mod sync;
mod unsubscribe;
mod translation_policy;
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use dakia_domain::*;
use dakia_storage::{Error as StorageError, Store};
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl,
};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Clone)]
pub struct App {
    pub store: Store,
    pub config: Arc<Config>,
    pub mail: mail::MailRuntime,
}
#[derive(Clone)]
pub struct Config {
    pub origin: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub web_root: String,
}
impl Config {
    pub fn validate(&self) -> std::result::Result<(), String> {
        for value in
            std::iter::once(&self.origin).chain((!self.issuer.is_empty()).then_some(&self.issuer))
        {
            let u = url::Url::parse(value).map_err(|_| "invalid URL")?;
            if u.scheme() != "https"
                || u.host_str().is_none()
                || !u.username().is_empty()
                || u.password().is_some()
                || u.query().is_some()
                || u.fragment().is_some()
            {
                return Err("HTTPS URLs required without credentials/query/fragment".into());
            }
        }
        if self.origin.ends_with('/')
            || (!self.issuer.is_empty()
                && (self.client_id.is_empty() || self.client_secret.is_empty()))
            || (self.issuer.is_empty()
                && (!self.client_id.is_empty() || !self.client_secret.is_empty()))
        {
            return Err("origin must not end in /; OIDC client configuration required".into());
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Identity {
    user: String,
    csrf: String,
    token_hash: String,
}
#[derive(Debug)]
struct ApiFailure(StatusCode, &'static str);
impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(ApiError {
                code: self.1.into(),
                message: match self.1 {
                    "content_reference_unavailable" => {
                        "This message’s content isn’t currently available from your mail provider."
                            .into()
                    }
                    "content_reference_changed" => {
                        "This message changed while loading. Try opening it again.".into()
                    }
                    "message_too_large" => {
                        "This message is too large to open here. Open it in your mail provider."
                            .into()
                    }
                    _ => self.1.replace('_', " "),
                },
            }),
        )
            .into_response()
    }
}
impl From<StorageError> for ApiFailure {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::NotFound => Self(StatusCode::NOT_FOUND, "not_found"),
            StorageError::Conflict => Self(StatusCode::CONFLICT, "revision_conflict"),
            StorageError::CleanupMoveUnverified => {
                Self(StatusCode::CONFLICT, "cleanup_move_unverified")
            }
            StorageError::CursorExpired => Self(StatusCode::CONFLICT, "refetch_required"),
            StorageError::QuotaExceeded => Self(
                StatusCode::INSUFFICIENT_STORAGE,
                "attachment_quota_exceeded",
            ),
            StorageError::Invalid => Self(StatusCode::BAD_REQUEST, "invalid_input"),
            StorageError::AccountBusy => Self(StatusCode::CONFLICT, "account_has_pending_actions"),
            _ => Self(StatusCode::INTERNAL_SERVER_ERROR, "storage_error"),
        }
    }
}
type Result<T> = std::result::Result<T, ApiFailure>;
fn internal(_: impl std::fmt::Display) -> ApiFailure {
    ApiFailure(StatusCode::BAD_GATEWAY, "identity_provider_unavailable")
}
fn random() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}
fn hash(s: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(s.as_bytes()))
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then(|| v.to_owned()))
}
fn session_cookie(value: &str, max_age: u32) -> String {
    format!("__Host-dakia={value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age}")
}
async fn identity(app: &App, token_hash: &str) -> Result<Identity> {
    let row = sqlx::query("SELECT user_id,csrf FROM sessions WHERE token_hash=? AND expires_at>?")
        .bind(token_hash)
        .bind(Utc::now().timestamp())
        .fetch_optional(app.store.control())
        .await
        .map_err(StorageError::from)?;
    let row = row.ok_or(ApiFailure(StatusCode::UNAUTHORIZED, "login_required"))?;
    Ok(Identity {
        user: row.get("user_id"),
        csrf: row.get("csrf"),
        token_hash: token_hash.into(),
    })
}
async fn protect(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let result = async {
        let token = cookie(req.headers(), "__Host-dakia")
            .ok_or(ApiFailure(StatusCode::UNAUTHORIZED, "login_required"))?;
        let id = identity(&app, &hash(&token)).await?;
        let mutation = !matches!(
            *req.method(),
            axum::http::Method::GET | axum::http::Method::HEAD
        );
        let valid_origin = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            == Some(&app.config.origin);
        let valid_csrf = req
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            == Some(&id.csrf);
        if mutation && !(valid_origin && valid_csrf) {
            return Err(ApiFailure(StatusCode::FORBIDDEN, "csrf_rejected"));
        }
        req.extensions_mut().insert(id);
        Ok(())
    }
    .await;
    match result {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}
pub fn router(app: App) -> Router {
    router_with_shutdown(app, tokio::sync::watch::channel(false).0)
}
pub fn router_with_shutdown(app: App, shutdown: tokio::sync::watch::Sender<bool>) -> Router {
    let api = Router::new()
        .route("/session", get(session))
        .route("/translation/settings", get(translation_policy::get_policy).put(translation_policy::set_policy))
        .route("/translation/models/{hash}", get(translation_policy::download))
        .route("/logout", post(logout))
        .route("/local-access", get(local_users::access))
        .route("/releases", get(releases::status))
        .route(
            "/invitations",
            get(local_users::list).post(local_users::create),
        )
        .route(
            "/invitations/{id}",
            axum::routing::delete(local_users::revoke),
        )
        .route("/rules", get(rules::list).post(rules::create))
        .route("/rules/{id}", axum::routing::patch(rules::update))
        .route("/accounts/{account}/rule-activity", get(rules::activity))
        .route("/accounts", get(accounts).post(connect))
        .route("/accounts/discover", post(discovery::discover))
        .route(
            "/accounts/gmail",
            get(gmail_oauth::status).post(gmail_oauth::start),
        )
        .route("/accounts/gmail/callback", get(gmail_oauth::callback))
        .route("/accounts/microsoft", get(microsoft_oauth::status).post(microsoft_oauth::start))
        .route("/accounts/microsoft/callback", get(microsoft_oauth::callback))
        .route("/push/public-key", get(push::public_key))
        .route("/push/subscriptions", post(push::subscribe))
        .route(
            "/accounts/{account}",
            axum::routing::put(reconnect).delete(remove_account),
        )
        .route("/search", get(search::search))
        .route(
            "/accounts/{account}/messages/{message}/sender-mail",
            get(sender_mail::cached),
        )
        .route("/accounts/{account}/folders", get(folders))
        .route("/accounts/{account}/sync-active", get(sync_active))
        .route(
            "/accounts/{account}/settings",
            get(mailbox_settings).put(save_mailbox_settings),
        )
        .route("/accounts/{account}/drafts", get(drafts))
        .route("/accounts/{account}/recipients", get(recipients))
        .route("/accounts/{account}/probe", post(probe_account))
        .route("/accounts/{account}/messages", get(messages))
        .route("/accounts/{account}/messages/{id}", get(content))
        .route(
            "/accounts/{account}/messages/{id}/notification",
            get(push::summary),
        )
        .route(
            "/accounts/{account}/messages/{id}/conversation",
            get(conversation),
        )
        .route(
            "/accounts/{account}/messages/{id}/unsubscribe",
            get(unsubscribe::info).post(unsubscribe::execute),
        )
        .route(
            "/accounts/{account}/messages/{id}/image",
            post(images::load),
        )
        .route(
            "/accounts/{account}/messages/{id}/translation",
            get(models::translated).post(models::queue),
        )
        .route("/accounts/{account}/jobs/{id}", get(models::job))
        .route(
            "/accounts/{account}/drafts/{id}",
            get(draft).put(save_draft),
        )
        .route(
            "/accounts/{account}/submissions",
            get(submissions).post(submit),
        )
        .route("/accounts/{account}/submissions/{id}", get(submission))
        .route("/accounts/{account}/scheduled", get(scheduled_messages))
        .route("/accounts/{account}/submissions/{id}/cancel", post(cancel))
        .route("/accounts/{account}/events", get(events))
        .route("/accounts/{account}/operations", post(operation))
        .route(
            "/accounts/{account}/sender-cleanups",
            get(sender_cleanup::list).post(sender_cleanup::enqueue),
        )
        .route(
            "/accounts/{account}/sender-cleanups/{id}/dismiss",
            post(sender_cleanup::dismiss),
        )
        .route("/accounts/{account}/operations/{id}", get(operation_status))
        .route("/accounts/{account}/attachments", post(upload))
        .route("/accounts/{account}/attachments/{id}", get(download))
        .fallback(|| async { ApiFailure(StatusCode::NOT_FOUND, "endpoint_not_available") })
        .route_layer(middleware::from_fn_with_state(app.clone(), protect));
    let root = app.config.web_root.clone();
    Router::new()
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"service":"dakia-web","status":"ok"})) }),
        )
        .route(
            "/auth/login",
            get(login)
                .post(local_auth::submit)
                .layer(axum::extract::DefaultBodyLimit::max(4096)),
        )
        .route(
            "/auth/join",
            get(local_auth::join_page)
                .post(local_auth::join_submit)
                .layer(axum::extract::DefaultBodyLimit::max(4096)),
        )
        .route("/auth/callback", get(callback))
        .route("/api/microsoft/notifications", post(graph_push::notify).layer(axum::extract::DefaultBodyLimit::max(64 * 1024)))
        .nest("/api", api)
        .fallback_service(
            ServeDir::new(&root).not_found_service(ServeFile::new(format!("{root}/index.html"))),
        )
        .layer(axum::extract::DefaultBodyLimit::max(22 * 1024 * 1024))
        .layer(Extension(shutdown))
        .layer(Extension(Arc::new(
            releases::ReleaseChecker::from_environment(),
        )))
        .layer(middleware::from_fn(|req: Request, next: Next| async move {
            let login_form = matches!(req.uri().path(), "/auth/login" | "/auth/join");
            let mut r = next.run(req).await;
            r.headers_mut()
                .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            r.headers_mut()
                .insert("x-content-type-options", "nosniff".parse().unwrap());
            r.headers_mut().insert(
                "referrer-policy",
                if login_form {
                    "same-origin"
                } else {
                    "no-referrer"
                }
                .parse()
                .unwrap(),
            );
            r
        }))
        .with_state(app)
}
async fn session(Extension(id): Extension<Identity>) -> Json<Session> {
    Json(Session {
        user_id: id.user,
        csrf_token: id.csrf,
    })
}
async fn probe_account(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<Account>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.mail.probe(&app.store, &a).await?))
}
#[derive(Deserialize)]
struct MailPage {
    folder: Option<String>,
    continuation: Option<String>,
    q: Option<String>,
}
async fn messages(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Query(query): Query<MailPage>,
) -> Result<Json<SearchPage>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(
        app.mail
            .page(
                &app.store,
                &a,
                query.folder.as_deref().unwrap_or("INBOX"),
                dakia_mail_provider::Search {
                    text: query.q,
                    ..Default::default()
                },
                query.continuation.as_deref(),
            )
            .await?,
    ))
}
#[tracing::instrument(
    target = "dakia_server::diagnostics",
    skip_all,
    fields(purpose = "content")
)]
async fn content(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<Json<MessageContent>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.mail.content(&app.store, &a, &id).await?))
}
#[derive(Deserialize)]
struct ConversationQuery {
    continuation: Option<String>,
    #[serde(default)]
    snapshot: bool,
    #[serde(default)]
    refresh: bool,
}
#[tracing::instrument(
    target = "dakia_server::diagnostics",
    skip_all,
    fields(purpose = "conversation")
)]
async fn conversation(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
    Query(query): Query<ConversationQuery>,
) -> Result<Json<ConversationPage>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(
        app.mail
            .conversation_read(
                &app.store,
                &a,
                &id,
                query.continuation.as_deref(),
                query.snapshot,
                query.refresh,
            )
            .await?,
    ))
}
async fn operation(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Json(op): Json<Operation>,
) -> Result<Json<Operation>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let result = app.store.operation(&a, op).await?;
    tracing::info!(target: "dakia_server::diagnostics",
        account_key = diagnostics::correlation(a.id()),
        operation_key = diagnostics::correlation(&result.id),
        message_key = diagnostics::correlation(&result.target_id),
        action = mutations::action_name(&result.change),
        status = ?result.status, stage = "accepted", "mail operation stored");
    Ok(Json(result))
}
async fn operation_status(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<Json<Operation>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.operation_status(&a, &id).await?))
}
async fn upload(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Result<Json<Attachment>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let filename = headers
        .get("x-filename")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("attachment.bin")
        .to_owned();
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|s| s.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    Ok(Json(
        app.store
            .put_attachment(&a, filename, content_type, &bytes)
            .await?,
    ))
}
#[tracing::instrument(
    target = "dakia_server::diagnostics",
    skip_all,
    fields(purpose = "attachment")
)]
async fn download(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<Response> {
    let a = app.store.authorize(&user.user, &account).await?;
    let (metadata, bytes) = match app.store.attachment(&a, &id).await {
        Ok(part) => part,
        Err(StorageError::NotFound) => app.mail.received_part(&app.store, &a, &id).await?,
        Err(e) => return Err(e.into()),
    };
    let disposition = attachment_disposition(&metadata.filename);
    // Never render untrusted attachment bytes in the application origin.
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
            (
                header::CONTENT_SECURITY_POLICY,
                "sandbox; default-src 'none'",
            ),
        ],
        bytes,
    )
        .into_response())
}

fn attachment_disposition(filename: &str) -> String {
    // Strip paths and controls, retain the extension, and use RFC 5987 for
    // Unicode. Sender-controlled text can never add another response header.
    let filename = filename.rsplit(['/', '\\']).next().unwrap_or("");
    let filename: String = filename
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    let filename = filename.trim();
    let filename = if filename.is_empty() || filename == "." || filename == ".." {
        "attachment.bin"
    } else {
        filename
    };
    let ascii: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " .-_()".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = filename
        .as_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b".-_".contains(b) {
                (*b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}
async fn logout(State(app): State<App>, Extension(id): Extension<Identity>) -> Result<Response> {
    sqlx::query("DELETE FROM sessions WHERE token_hash=?")
        .bind(id.token_hash)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    Ok((
        [(header::SET_COOKIE, session_cookie("", 0))],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
async fn accounts(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
) -> Result<Json<Vec<Account>>> {
    Ok(Json(app.store.accounts(&id.user).await?))
}
#[derive(Deserialize)]
struct RemoveAccount {
    confirm_email: String,
}
async fn remove_account(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Json(input): Json<RemoveAccount>,
) -> Result<StatusCode> {
    let a = app.store.authorize(&user.user, &account).await?;
    app.mail
        .remove_account(&app.store, &a, &input.confirm_email)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn folders(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<Vec<String>>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(
        app.mail
            .folders(&app.store, &a)
            .await?
            .into_iter()
            .map(|f| f.name)
            .collect(),
    ))
}
async fn sync_active(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<bool>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.mail.sync_active(&a).await))
}
async fn drafts(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<Vec<Draft>>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.drafts(&a).await?))
}
async fn submissions(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<Vec<Submission>>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.submissions(&a).await?))
}
async fn scheduled_messages(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<Vec<ScheduledMessage>>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.scheduled_messages(&a).await?))
}
async fn recipients(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Query(query): Query<RecipientQuery>,
) -> Result<Json<Vec<Address>>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let needle = query.q.unwrap_or_default();
    let mut addresses = app.store.recipient_suggestions(&a, &needle).await?;
    // Account-local evidence ranks first; other accounts must belong to this user.
    for account in app.store.accounts(&user.user).await? {
        if account.id == a.id() || addresses.len() >= 20 {
            continue;
        }
        let other = app.store.authorize(&user.user, &account.id).await?;
        for address in app.store.recipient_suggestions(&other, &needle).await? {
            if addresses.len() == 20 {
                break;
            }
            if !addresses
                .iter()
                .any(|v| v.email.eq_ignore_ascii_case(&address.email))
            {
                addresses.push(address);
            }
        }
    }
    Ok(Json(addresses))
}
async fn mailbox_settings(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<MailboxSettings>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.mail.settings(&app.store, &a).await?))
}
async fn save_mailbox_settings(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Json(settings): Json<MailboxSettings>,
) -> Result<Json<MailboxSettings>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.mail.configure(&app.store, &a, settings).await?))
}
#[derive(Deserialize)]
struct RecipientQuery {
    q: Option<String>,
}
async fn connect(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Json(input): Json<ConnectAccount>,
) -> Result<Json<Account>> {
    if input.gmail_oauth.is_some() || input.microsoft_oauth.is_some() || input.provider == ProviderKind::Microsoft {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_input"));
    }
    Ok(Json(app.store.connect(&id.user, &input).await?))
}
async fn reconnect(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Json(input): Json<ConnectAccount>,
) -> Result<Json<Account>> {
    let a = app.store.authorize(&user.user, &account).await?;
    if input.gmail_oauth.is_some() || input.microsoft_oauth.is_some() || input.provider == ProviderKind::Microsoft {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_input"));
    }
    Ok(Json(app.mail.reconnect(&app.store, &a, &input).await?))
}
async fn draft(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<Json<Draft>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.draft(&a, &id).await?))
}
async fn save_draft(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
    Json(mut d): Json<Draft>,
) -> Result<Json<Draft>> {
    let a = app.store.authorize(&user.user, &account).await?;
    if d.id != id || d.account_id != a.id() || d.attachments.len() > 100 {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "id_mismatch"));
    }
    // Materialize only explicitly selected received parts. Browser metadata is not trusted.
    for part in &mut d.attachments {
        *part = match app.store.attachment(&a, &part.id).await {
            Ok((metadata, _)) => metadata,
            Err(StorageError::NotFound) => {
                app.mail
                    .pin_received_attachment(&app.store, &a, &part.id)
                    .await?
            }
            Err(error) => return Err(error.into()),
        };
    }
    let draft = app.store.save_draft(&a, d).await?;
    let addresses: Vec<_> = draft
        .to
        .iter()
        .chain(&draft.cc)
        .chain(&draft.bcc)
        .cloned()
        .collect();
    app.store
        .learn_recipients(
            &a,
            &format!("draft:{}", draft.id),
            &addresses,
            &draft.saved_at,
        )
        .await?;
    Ok(Json(draft))
}
async fn submit(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Json(r): Json<SubmitRequest>,
) -> Result<Json<Submission>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.submit(&a, &r).await?))
}
async fn submission(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<Json<Submission>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.submission(&a, &id).await?))
}
async fn cancel(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<Json<Submission>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let s = app.store.submission(&a, &id).await?;
    Ok(Json(
        app.store
            .transition(&a, &id, s.state, SubmissionState::Cancelled)
            .await?,
    ))
}
#[derive(Deserialize)]
struct Replay {
    #[serde(default)]
    after: u32,
}
async fn events(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Extension(shutdown): Extension<tokio::sync::watch::Sender<bool>>,
    Path(account): Path<String>,
    Query(replay): Query<Replay>,
) -> Result<Response> {
    let a = app.store.authorize(&user.user, &account).await?;
    // Snapshot before the ready-triggered refetch, not the entire history.
    let latest = app.store.event_cursor(&a).await?;
    let mut cursor = replay.after;
    let reset = cursor == 0
        || cursor > latest
        || latest.saturating_sub(cursor) > 500
        || matches!(
            app.store.events(&a, cursor).await,
            Err(StorageError::CursorExpired)
        );
    if reset {
        cursor = latest;
    }
    app.store.events(&a, cursor).await?;
    let mut stopping = shutdown.subscribe();
    let stream = async_stream::stream! {
        yield Ok::<_,std::convert::Infallible>(axum::response::sse::Event::default()
            .event("ready").id(cursor.to_string())
            .data(serde_json::json!({"cursor": cursor, "reset": reset}).to_string()));
        loop {
            if *stopping.borrow() {break}
            if identity(&app,&user.token_hash).await.is_err(){break}
            match app.store.events(&a,cursor).await {
                Ok(events)=>for e in events {cursor=e.cursor;let json=serde_json::to_string(&e).unwrap();yield Ok::<_,std::convert::Infallible>(axum::response::sse::Event::default().id(cursor.to_string()).data(json));},
                Err(_)=>{yield Ok(axum::response::sse::Event::default().event("refetch_required").data("{}"));break}
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                _ = stopping.changed() => {break},
            }
        }
    };
    Ok(axum::response::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}
fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(internal)
}
async fn metadata(app: &App, http: &reqwest::Client) -> Result<CoreProviderMetadata> {
    CoreProviderMetadata::discover_async(
        IssuerUrl::new(app.config.issuer.clone()).map_err(internal)?,
        http,
    )
    .await
    .map_err(internal)
}
async fn login(State(app): State<App>) -> Result<Response> {
    if app.config.issuer.is_empty() {
        return local_auth::page(&app).await;
    }
    let http = http()?;
    let meta = metadata(&app, &http).await?;
    let client = CoreClient::from_provider_metadata(
        meta,
        ClientId::new(app.config.client_id.clone()),
        Some(ClientSecret::new(app.config.client_secret.clone())),
    )
    .set_redirect_uri(
        RedirectUrl::new(format!("{}/auth/callback", app.config.origin)).map_err(internal)?,
    );
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (url, state, nonce) = client
        .authorize_url(
            AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(challenge)
        .url();
    sqlx::query("DELETE FROM oidc_flows WHERE expires_at<=?")
        .bind(Utc::now().timestamp())
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    sqlx::query("INSERT INTO oidc_flows(state_hash,nonce,verifier,expires_at) VALUES(?,?,?,?)")
        .bind(hash(state.secret()))
        .bind(nonce.secret())
        .bind(verifier.secret())
        .bind(Utc::now().timestamp() + 600)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    // Browser-bound state prevents login CSRF, database deletion prevents replay.
    let cookie = format!(
        "__Host-dakia-flow={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=600",
        state.secret()
    );
    Ok((
        [(header::SET_COOKIE, cookie)],
        Redirect::temporary(url.as_str()),
    )
        .into_response())
}
#[derive(Deserialize)]
struct Callback {
    code: String,
    state: String,
}
async fn callback(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<Callback>,
) -> Result<Response> {
    if app.config.issuer.is_empty() {
        return Err(ApiFailure(StatusCode::NOT_FOUND, "oidc_disabled"));
    }
    if cookie(&headers, "__Host-dakia-flow").as_deref() != Some(&q.state) {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_login_state"));
    }
    let flow = sqlx::query(
        "DELETE FROM oidc_flows WHERE state_hash=? AND expires_at>? RETURNING nonce,verifier",
    )
    .bind(hash(&q.state))
    .bind(Utc::now().timestamp())
    .fetch_optional(app.store.control())
    .await
    .map_err(StorageError::from)?
    .ok_or(ApiFailure(StatusCode::BAD_REQUEST, "invalid_login_state"))?;
    let http = http()?;
    let meta = metadata(&app, &http).await?;
    let client = CoreClient::from_provider_metadata(
        meta,
        ClientId::new(app.config.client_id.clone()),
        Some(ClientSecret::new(app.config.client_secret.clone())),
    )
    .set_redirect_uri(
        RedirectUrl::new(format!("{}/auth/callback", app.config.origin)).map_err(internal)?,
    );
    let tokens = client
        .exchange_code(AuthorizationCode::new(q.code))
        .map_err(internal)?
        .set_pkce_verifier(PkceCodeVerifier::new(flow.get("verifier")))
        .request_async(&http)
        .await
        .map_err(internal)?;
    let id_token = tokens.extra_fields().id_token().ok_or(ApiFailure(
        StatusCode::BAD_GATEWAY,
        "missing_identity_token",
    ))?;
    let verifier = client.id_token_verifier();
    let claims = id_token
        .claims(&verifier, &Nonce::new(flow.get("nonce")))
        .map_err(internal)?;
    let user = app
        .store
        .user(&app.config.issuer, claims.subject().as_str())
        .await?;
    let token = random();
    let csrf = random();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(hash(&token))
        .bind(user)
        .bind(csrf)
        .bind(Utc::now().timestamp() + 86400 * 7)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        session_cookie(&token, 86400 * 7).parse().unwrap(),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        "__Host-dakia-flow=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0"
            .parse()
            .unwrap(),
    );
    Ok(response)
}

pub use local_auth::{initialize_credential_key, initialize_local_login};
