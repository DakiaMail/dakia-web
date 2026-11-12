//! Operator-owned Microsoft Graph authorization, separate from Dakia login.
#[cfg(test)]
#[path = "microsoft_oauth_tests.rs"]
mod tests;
use super::*;
use dakia_mail_provider::{self as p, AccessToken};
use dakia_storage::AuthorizedAccount;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use uuid::Uuid;

const SCOPES: &str = "openid email profile offline_access https://graph.microsoft.com/User.Read https://graph.microsoft.com/Mail.ReadWrite https://graph.microsoft.com/Mail.Send";
#[derive(Clone)]
pub struct MicrosoftConfig {
    pub client_id: String,
    pub client_secret: String,
    pub tenant: String,
}
impl MicrosoftConfig {
    pub fn from_env() -> std::result::Result<Option<Self>, Box<dyn std::error::Error>> {
        let client_id = std::env::var("DAKIA_MICROSOFT_CLIENT_ID").unwrap_or_default();
        let client_secret = match std::env::var("DAKIA_MICROSOFT_SECRET_FILE") {
            Ok(path) => std::fs::read_to_string(path)?.trim().to_owned(),
            Err(_) => std::env::var("DAKIA_MICROSOFT_CLIENT_SECRET").unwrap_or_default(),
        };
        if client_id.is_empty() && client_secret.is_empty() {
            return Ok(None);
        }
        let tenant = std::env::var("DAKIA_MICROSOFT_TENANT").unwrap_or_else(|_| "common".into());
        if client_id.trim().is_empty() || client_secret.trim().is_empty() || !valid_tenant(&tenant)
        {
            return Err("Microsoft requires a client ID, secret and valid tenant (common, organizations, consumers or tenant GUID)".into());
        }
        Ok(Some(Self {
            client_id,
            client_secret,
            tenant,
        }))
    }
    fn endpoint(&self, action: &str) -> String {
        format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/{action}",
            self.tenant
        )
    }
}
fn valid_tenant(tenant: &str) -> bool {
    matches!(tenant, "common" | "organizations" | "consumers") || Uuid::parse_str(tenant).is_ok()
}
fn configured(app: &App) -> Result<&MicrosoftConfig> {
    app.mail.microsoft.as_ref().ok_or(ApiFailure(
        StatusCode::SERVICE_UNAVAILABLE,
        "microsoft_sign_in_not_configured",
    ))
}
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .expect("HTTP client")
}
fn unavailable(_: impl std::fmt::Display) -> ApiFailure {
    ApiFailure(StatusCode::BAD_GATEWAY, "microsoft_sign_in_unavailable")
}
pub(super) async fn status(State(app): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"enabled": app.mail.microsoft.is_some()}))
}
#[derive(Deserialize)]
pub(super) struct Start {
    email: String,
    display_name: String,
    account_id: Option<String>,
}
pub(super) async fn start(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Json(input): Json<Start>,
) -> Result<Json<serde_json::Value>> {
    let config = configured(&app)?;
    if input.email.len() > 254
        || !input.email.contains('@')
        || input.email.contains(char::is_whitespace)
        || input.display_name.trim().is_empty()
        || input.display_name.len() > 200
    {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_input"));
    }
    if let Some(account) = &input.account_id {
        let a = app.store.authorize(&id.user, account).await?;
        let old = app.store.credential(&a).await?;
        if old.provider != ProviderKind::Microsoft || !old.email.eq_ignore_ascii_case(&input.email)
        {
            return Err(ApiFailure(StatusCode::BAD_REQUEST, "account_mismatch"));
        }
    }
    let state = random();
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let now = Utc::now().timestamp();
    sqlx::query("DELETE FROM microsoft_oauth_flows WHERE expires_at<=? OR session_hash=?")
        .bind(now)
        .bind(&id.token_hash)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    sqlx::query("INSERT INTO microsoft_oauth_flows(state_hash,session_hash,user_id,verifier,email,display_name,account_id,expires_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(hash(&state)).bind(&id.token_hash).bind(&id.user).bind(verifier.secret())
        .bind(&input.email).bind(&input.display_name).bind(&input.account_id).bind(now+600)
        .execute(app.store.control()).await.map_err(StorageError::from)?;
    let mut url = url::Url::parse(&config.endpoint("authorize")).map_err(unavailable)?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", config.client_id.as_str()),
        (
            "redirect_uri",
            &format!("{}/api/accounts/microsoft/callback", app.config.origin),
        ),
        ("response_type", "code"),
        ("response_mode", "query"),
        ("scope", SCOPES),
        ("prompt", "consent"),
        ("state", &state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("login_hint", &input.email),
    ]);
    Ok(Json(serde_json::json!({"url":url.as_str()})))
}
#[derive(Deserialize)]
pub(super) struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    token_type: String,
    scope: Option<String>,
}
fn valid_tokens(tokens: &Tokens) -> bool {
    !tokens.access_token.is_empty()
        && !tokens.access_token.contains(['\r', '\n', '\x01'])
        && tokens.token_type.eq_ignore_ascii_case("Bearer")
        && tokens.expires_in > 60
}
fn mail_permissions(tokens: &Tokens) -> bool {
    // OAuth omits scope only when the grant matches the requested set.
    let scope = tokens.scope.as_deref().unwrap_or(SCOPES);
    ["User.Read", "Mail.ReadWrite", "Mail.Send"]
        .iter()
        .all(|required| {
            scope.split_whitespace().any(|s| {
                s.strip_prefix("https://graph.microsoft.com/")
                    .unwrap_or(s)
                    .eq_ignore_ascii_case(required)
            })
        })
}
async fn complete(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Query(input): Query<Callback>,
) -> Result<Redirect> {
    let config = configured(&app)?;
    let token_url = config.endpoint("token");
    let account = authorize(
        &app,
        &id,
        input,
        &token_url,
        "https://graph.microsoft.com/v1.0/me?$select=id,mail,userPrincipalName",
    )
    .await?;
    app.mail.probe(&app.store, &account).await?;
    Ok(Redirect::to("/"))
}
async fn authorize(
    app: &App,
    id: &Identity,
    input: Callback,
    token_url: &str,
    profile_url: &str,
) -> Result<dakia_storage::AuthorizedAccount> {
    let config = configured(app)?;
    let row = sqlx::query("DELETE FROM microsoft_oauth_flows WHERE state_hash=? AND session_hash=? AND user_id=? AND expires_at>? RETURNING *")
        .bind(hash(&input.state)).bind(&id.token_hash).bind(&id.user).bind(Utc::now().timestamp())
        .fetch_optional(app.store.control()).await.map_err(StorageError::from)?
        .ok_or(ApiFailure(StatusCode::BAD_REQUEST, "microsoft_sign_in_expired"))?;
    if let Some(error) = input.error {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            if error == "access_denied" {
                "microsoft_sign_in_cancelled"
            } else {
                "microsoft_sign_in_failed"
            },
        ));
    }
    let code = input.code.filter(|s| !s.is_empty()).ok_or(ApiFailure(
        StatusCode::BAD_REQUEST,
        "microsoft_sign_in_cancelled",
    ))?;
    let tokens: Tokens = http()
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &config.client_id),
            ("client_secret", &config.client_secret),
            ("scope", SCOPES),
            (
                "redirect_uri",
                &format!("{}/api/accounts/microsoft/callback", app.config.origin),
            ),
            ("code_verifier", &row.get::<String, _>("verifier")),
        ])
        .send()
        .await
        .map_err(unavailable)?
        .error_for_status()
        .map_err(unavailable)?
        .json()
        .await
        .map_err(unavailable)?;
    if !valid_tokens(&tokens) || !mail_permissions(&tokens) {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "microsoft_permission_required",
        ));
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Profile {
        id: String,
        mail: Option<String>,
        user_principal_name: Option<String>,
    }
    let profile: Profile = http()
        .get(profile_url)
        .bearer_auth(&tokens.access_token)
        .send()
        .await
        .map_err(unavailable)?
        .error_for_status()
        .map_err(unavailable)?
        .json()
        .await
        .map_err(unavailable)?;
    let email: String = row.get("email");
    let account: Option<String> = row.get("account_id");
    if profile.id.is_empty()
        || (account.is_none()
            && ![
                profile.mail.as_deref(),
                profile.user_principal_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|s| s.eq_ignore_ascii_case(&email)))
    {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "microsoft_account_mismatch",
        ));
    }
    let mailbox = profile
        .mail
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(profile.user_principal_name.as_deref())
        .filter(|s| valid_email(s))
        .ok_or(ApiFailure(
            StatusCode::BAD_REQUEST,
            "microsoft_account_mismatch",
        ))?
        .to_owned();
    if let Some(account) = &account {
        let a = app.store.authorize(&id.user, account).await?;
        let old = app.store.credential(&a).await?;
        if old.microsoft_oauth.is_none_or(|g| g.subject != profile.id) {
            return Err(ApiFailure(
                StatusCode::BAD_REQUEST,
                "microsoft_account_mismatch",
            ));
        }
    }
    let refresh_token = tokens
        .refresh_token
        .filter(|s| !s.is_empty())
        .ok_or(ApiFailure(
            StatusCode::BAD_REQUEST,
            "microsoft_offline_access_required",
        ))?;
    let connection = ConnectAccount {
        email: mailbox.clone(),
        display_name: row.get("display_name"),
        provider: ProviderKind::Microsoft,
        // Kept for the shared connection schema; Graph never opens these endpoints.
        imap_host: "graph.microsoft.com".into(),
        imap_port: 443,
        imap_tls: TlsMode::Implicit,
        smtp_host: "graph.microsoft.com".into(),
        smtp_port: 443,
        smtp_tls: TlsMode::Implicit,
        username: mailbox,
        password: String::new(),
        gmail_oauth: None,
        microsoft_oauth: Some(MicrosoftGrant {
            client_id: config.client_id.clone(),
            tenant: config.tenant.clone(),
            subject: profile.id,
            refresh_token,
        }),
    };
    let account = if let Some(account) = account {
        let a = app.store.authorize(&id.user, &account).await?;
        app.mail.reconnect(&app.store, &a, &connection).await?
    } else {
        app.store.connect(&id.user, &connection).await?
    };
    Ok(app.store.authorize(&id.user, &account.id).await?)
}
fn valid_email(email: &str) -> bool {
    email.len() <= 254
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        })
        && !email.chars().any(|c| c.is_control() || c.is_whitespace())
}
pub(super) async fn callback(
    state: State<App>,
    id: Extension<Identity>,
    query: Query<Callback>,
) -> Redirect {
    match complete(state, id, query).await {
        Ok(redirect) => redirect,
        Err(error) => {
            let reason = match error.1 {
                "microsoft_sign_in_cancelled" => "cancelled",
                "microsoft_sign_in_expired" => "expired",
                "microsoft_account_mismatch" => "mismatch",
                "microsoft_permission_required" => "permission",
                "microsoft_offline_access_required" => "offline",
                _ => "failed",
            };
            Redirect::to(&format!("/?microsoft_error={reason}"))
        }
    }
}
pub(crate) struct MicrosoftToken {
    config: MicrosoftConfig,
    store: Store,
    account: AuthorizedAccount,
    grant: Mutex<MicrosoftGrant>,
    cache: Mutex<Option<(Instant, String)>>,
    token_url: String,
}
impl MicrosoftToken {
    pub(crate) fn new(
        config: MicrosoftConfig,
        grant: MicrosoftGrant,
        store: Store,
        account: AuthorizedAccount,
    ) -> Self {
        let token_url = config.endpoint("token");
        Self {
            config,
            store,
            account,
            grant: Mutex::new(grant),
            cache: Mutex::new(None),
            token_url,
        }
    }
}
#[async_trait::async_trait]
impl AccessToken for MicrosoftToken {
    async fn access_token(&self) -> p::Result<String> {
        let mut cache = self.cache.lock().await;
        if let Some((expiry, token)) = cache.as_ref() {
            if *expiry > Instant::now() {
                return Ok(token.clone());
            }
        }
        let mut grant = self.grant.lock().await;
        let response = http()
            .post(&self.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", grant.refresh_token.as_str()),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
                ("scope", SCOPES),
            ])
            .send()
            .await
            .map_err(|_| {
                p::error(
                    p::ErrorKind::Retryable,
                    "Microsoft token service unavailable",
                )
            })?;
        if !response.status().is_success() {
            #[derive(Deserialize)]
            struct Failure {
                error: String,
            }
            let failure = response.json::<Failure>().await.ok();
            let kind = if failure.is_some_and(|e| {
                matches!(
                    e.error.as_str(),
                    "invalid_grant"
                        | "invalid_client"
                        | "unauthorized_client"
                        | "interaction_required"
                        | "consent_required"
                )
            }) {
                p::ErrorKind::Authentication
            } else {
                p::ErrorKind::Retryable
            };
            return Err(p::error(kind, "Microsoft token renewal failed"));
        }
        let tokens: Tokens = response
            .json()
            .await
            .map_err(|_| p::error(p::ErrorKind::Retryable, "Microsoft token response invalid"))?;
        if !valid_tokens(&tokens) || !mail_permissions(&tokens) {
            return Err(p::error(
                p::ErrorKind::Authentication,
                "Microsoft mail permissions missing",
            ));
        }
        if let Some(new) = tokens.refresh_token.filter(|s| !s.is_empty()) {
            self.store
                .rotate_microsoft_token(&self.account, &grant.refresh_token, &new)
                .await
                .map_err(|_| {
                    p::error(
                        p::ErrorKind::Authentication,
                        "Microsoft token persistence failed",
                    )
                })?;
            grant.refresh_token = new;
        }
        *cache = Some((
            Instant::now() + Duration::from_secs(tokens.expires_in.min(86400) - 60),
            tokens.access_token.clone(),
        ));
        Ok(tokens.access_token)
    }
}
