//! Operator-owned Google mail authorization, separate from Dakia login.
use super::*;
use dakia_mail_provider::{self as p, AccessToken};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const MAIL_SCOPE: &str = "https://mail.google.com/";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Clone)]
pub struct GmailConfig {
    pub client_id: String,
    pub client_secret: String,
}
impl GmailConfig {
    pub fn from_env() -> std::result::Result<Option<Self>, Box<dyn std::error::Error>> {
        let client_id = std::env::var("DAKIA_GMAIL_CLIENT_ID").unwrap_or_default();
        let client_secret = match std::env::var("DAKIA_GMAIL_SECRET_FILE") {
            Ok(path) => std::fs::read_to_string(path)?.trim().to_owned(),
            Err(_) => std::env::var("DAKIA_GMAIL_CLIENT_SECRET").unwrap_or_default(),
        };
        if client_id.is_empty() && client_secret.is_empty() {
            return Ok(None);
        }
        if client_id.trim().is_empty() || client_secret.trim().is_empty() {
            return Err("Gmail requires DAKIA_GMAIL_CLIENT_ID and a client secret".into());
        }
        Ok(Some(Self {
            client_id,
            client_secret,
        }))
    }
}
fn configured(app: &App) -> Result<&GmailConfig> {
    app.mail.gmail.as_ref().ok_or(ApiFailure(
        StatusCode::SERVICE_UNAVAILABLE,
        "gmail_sign_in_not_configured",
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
    ApiFailure(StatusCode::BAD_GATEWAY, "google_sign_in_unavailable")
}
pub(super) async fn status(State(app): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"enabled": app.mail.gmail.is_some()}))
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
        if old.provider != ProviderKind::Gmail || !old.email.eq_ignore_ascii_case(&input.email) {
            return Err(ApiFailure(StatusCode::BAD_REQUEST, "account_mismatch"));
        }
    }
    let state = random();
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let now = Utc::now().timestamp();
    sqlx::query("DELETE FROM gmail_oauth_flows WHERE expires_at<=? OR session_hash=?")
        .bind(now)
        .bind(&id.token_hash)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    sqlx::query("INSERT INTO gmail_oauth_flows(state_hash,session_hash,user_id,verifier,email,display_name,account_id,expires_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(hash(&state)).bind(&id.token_hash).bind(&id.user).bind(verifier.secret())
        .bind(&input.email).bind(&input.display_name).bind(&input.account_id).bind(now+600)
        .execute(app.store.control()).await.map_err(StorageError::from)?;
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", config.client_id.as_str()),
        (
            "redirect_uri",
            &format!("{}/api/accounts/gmail/callback", app.config.origin),
        ),
        ("response_type", "code"),
        ("scope", &format!("openid email {MAIL_SCOPE}")),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("state", &state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("login_hint", &input.email),
    ]);
    Ok(Json(serde_json::json!({"url": url.as_str()})))
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
async fn complete(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Query(input): Query<Callback>,
) -> Result<Redirect> {
    let config = configured(&app)?;
    // Atomically consume only a flow belonging to this exact signed-in session.
    let row = sqlx::query("DELETE FROM gmail_oauth_flows WHERE state_hash=? AND session_hash=? AND user_id=? AND expires_at>? RETURNING *")
        .bind(hash(&input.state)).bind(&id.token_hash).bind(&id.user).bind(Utc::now().timestamp())
        .fetch_optional(app.store.control()).await.map_err(StorageError::from)?
        .ok_or(ApiFailure(StatusCode::BAD_REQUEST, "google_sign_in_expired"))?;
    if input.error.is_some() {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "google_sign_in_cancelled",
        ));
    }
    let code = input.code.filter(|c| !c.is_empty()).ok_or(ApiFailure(
        StatusCode::BAD_REQUEST,
        "google_sign_in_cancelled",
    ))?;
    let response = http()
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &config.client_id),
            ("client_secret", &config.client_secret),
            (
                "redirect_uri",
                &format!("{}/api/accounts/gmail/callback", app.config.origin),
            ),
            ("code_verifier", &row.get::<String, _>("verifier")),
        ])
        .send()
        .await
        .map_err(unavailable)?
        .error_for_status()
        .map_err(unavailable)?;
    let tokens: Tokens = response.json().await.map_err(unavailable)?;
    if !valid_tokens(&tokens)
        || !tokens
            .scope
            .as_deref()
            .is_some_and(|s| s.split_whitespace().any(|s| s == MAIL_SCOPE))
    {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "gmail_permission_required",
        ));
    }
    #[derive(Deserialize)]
    struct Profile {
        email: String,
        email_verified: bool,
    }
    let profile: Profile = http()
        .get("https://openidconnect.googleapis.com/v1/userinfo")
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
    if !profile.email_verified || !profile.email.eq_ignore_ascii_case(&email) {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "google_account_mismatch",
        ));
    }
    let refresh_token = tokens
        .refresh_token
        .filter(|s| !s.is_empty())
        .ok_or(ApiFailure(
            StatusCode::BAD_REQUEST,
            "google_offline_access_required",
        ))?;
    let connection = ConnectAccount {
        email,
        display_name: row.get("display_name"),
        provider: ProviderKind::Gmail,
        imap_host: "imap.gmail.com".into(),
        imap_port: 993,
        imap_tls: TlsMode::Implicit,
        smtp_host: "smtp.gmail.com".into(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        username: profile.email,
        password: String::new(),
        gmail_oauth: Some(GmailGrant {
            client_id: config.client_id.clone(),
            refresh_token,
        }),
    };
    let account: Option<String> = row.get("account_id");
    let account = if let Some(account) = account {
        let a = app.store.authorize(&id.user, &account).await?;
        app.mail.reconnect(&app.store, &a, &connection).await?
    } else {
        app.store.connect(&id.user, &connection).await?
    };
    let a = app.store.authorize(&id.user, &account.id).await?;
    // Probe never sends a message. A failure leaves the saved account available to reconnect.
    app.mail.probe(&app.store, &a).await?;
    Ok(Redirect::to("/"))
}

pub(crate) struct GoogleToken {
    config: GmailConfig,
    grant: GmailGrant,
    cache: Mutex<Option<(Instant, String)>>,
    token_url: String,
}
impl GoogleToken {
    pub(crate) fn new(config: GmailConfig, grant: GmailGrant) -> Self {
        Self {
            config,
            grant,
            cache: Mutex::new(None),
            token_url: TOKEN_URL.into(),
        }
    }
}
#[async_trait::async_trait]
impl AccessToken for GoogleToken {
    async fn access_token(&self) -> p::Result<String> {
        let mut cache = self.cache.lock().await;
        if let Some((expiry, token)) = cache.as_ref() {
            if *expiry > Instant::now() {
                return Ok(token.clone());
            }
        }
        let response = http()
            .post(&self.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", self.grant.refresh_token.as_str()),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
            ])
            .send()
            .await
            .map_err(|_| p::error(p::ErrorKind::Retryable, "Google token service unavailable"))?;
        if !response.status().is_success() {
            #[derive(Deserialize)]
            struct Failure {
                error: String,
            }
            let failure = response.json::<Failure>().await.ok();
            let kind = if failure.is_some_and(|e| {
                matches!(
                    e.error.as_str(),
                    "invalid_grant" | "invalid_client" | "unauthorized_client"
                )
            }) {
                p::ErrorKind::Authentication
            } else {
                p::ErrorKind::Retryable
            };
            return Err(p::error(kind, "Google token renewal failed"));
        }
        let tokens: Tokens = response
            .json()
            .await
            .map_err(|_| p::error(p::ErrorKind::Retryable, "Google token response invalid"))?;
        if !valid_tokens(&tokens) {
            return Err(p::error(
                p::ErrorKind::Retryable,
                "Google token response invalid",
            ));
        }
        *cache = Some((
            Instant::now() + Duration::from_secs(tokens.expires_in.min(86400) - 60),
            tokens.access_token.clone(),
        ));
        Ok(tokens.access_token)
    }
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
                "google_sign_in_cancelled" => "cancelled",
                "google_sign_in_expired" => "expired",
                "google_account_mismatch" => "mismatch",
                "gmail_permission_required" => "permission",
                "google_offline_access_required" => "offline",
                _ => "failed",
            };
            Redirect::to(&format!("/?gmail_error={reason}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn renewal_is_single_flight_cached_and_refreshes_before_expiry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let server = Router::new().route("/token", post(move |axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String,String>>| {
            let counter = counter.clone();
            async move {
                assert_eq!(form["grant_type"], "refresh_token");
                assert_eq!(form["refresh_token"], "fixture-refresh");
                assert_eq!(form["client_id"], "fixture-client");
                assert_eq!(form["client_secret"], "fixture-secret");
                let n = counter.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({"access_token":format!("token-{n}"),"expires_in":3600,"token_type":"Bearer"}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let mut source = GoogleToken::new(
            GmailConfig {
                client_id: "fixture-client".into(),
                client_secret: "fixture-secret".into(),
            },
            GmailGrant {
                client_id: "fixture-client".into(),
                refresh_token: "fixture-refresh".into(),
            },
        );
        source.token_url = format!("http://{addr}/token");
        let (a, b) = tokio::join!(source.access_token(), source.access_token());
        assert_eq!(a.unwrap(), "token-0");
        assert_eq!(b.unwrap(), "token-0");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        source.cache.lock().await.as_mut().unwrap().0 = Instant::now();
        assert_eq!(source.access_token().await.unwrap(), "token-1");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        task.abort();
    }

    #[tokio::test]
    async fn revoked_refresh_requires_reconnect_but_service_errors_are_retryable() {
        for (status, body, expected) in [
            (
                StatusCode::BAD_REQUEST,
                r#"{"error":"invalid_grant","error_description":"private-provider-detail"}"#,
                p::ErrorKind::Authentication,
            ),
            (
                StatusCode::UNAUTHORIZED,
                r#"{"error":"invalid_client"}"#,
                p::ErrorKind::Authentication,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                r#"{"error":"temporarily_unavailable"}"#,
                p::ErrorKind::Retryable,
            ),
        ] {
            let server = Router::new().route("/token", post(move || async move { (status, body) }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
            let mut source = GoogleToken::new(
                GmailConfig {
                    client_id: "fixture-client".into(),
                    client_secret: "fixture-secret".into(),
                },
                GmailGrant {
                    client_id: "fixture-client".into(),
                    refresh_token: "fixture-refresh".into(),
                },
            );
            source.token_url = format!("http://{addr}/token");
            let error = source.access_token().await.unwrap_err();
            assert_eq!(error.kind, expected);
            assert!(!error.message.contains("private"));
            assert!(source.cache.lock().await.is_none());
            task.abort();
        }
    }
}
