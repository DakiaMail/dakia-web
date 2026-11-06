use super::*;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{extract::Form, response::Html};
use std::{fs::OpenOptions, io::Write, path::Path};

fn private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub fn initialize_credential_key(data: &str, path: &str) -> std::io::Result<()> {
    if Path::new(path).exists() {
        return Ok(());
    }
    // A missing key on an existing installation must never be silently replaced.
    if Path::new(data).join("control.sqlite").exists() {
        return Err(std::io::Error::other(
            "existing data requires its original credential key",
        ));
    }
    let mut bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    private_file(Path::new(path), &bytes)
}

pub async fn initialize_local_login(
    store: &Store,
    key_path: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(store.control())
        .await?;
    let local: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_login")
        .fetch_one(store.control())
        .await?;
    if count > 0 && local == 0 {
        return Err(
            "existing OIDC users require OIDC configuration; local login cannot claim their data"
                .into(),
        );
    }
    if local == 0 {
        let path = Path::new(key_path).with_file_name("setup-token");
        let token = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let t = random();
                private_file(&path, t.as_bytes())?;
                t
            }
            Err(e) => return Err(e.into()),
        };
        if token.len() < 32 {
            return Err("invalid setup token file".into());
        }
        sqlx::query("INSERT INTO settings(key,value) VALUES('local_setup_token',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(hash(&token)).execute(store.control()).await?;
        tracing::info!("Local setup awaits the private setup-token file beside the credential key");
    }
    Ok(())
}

pub async fn page(app: &App) -> Result<Response> {
    render(app, "").await
}
async fn render(app: &App, error: &str) -> Result<Response> {
    let ready: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_login")
        .fetch_one(app.store.control())
        .await
        .map_err(StorageError::from)?;
    let title = if ready == 0 {
        "Set up Dakia"
    } else {
        "Sign in to Dakia"
    };
    let setup = if ready == 0 {
        "<label>Setup code<input name=\"code\" required autocomplete=\"off\" maxlength=\"256\"></label><p>Read the setup-token file on your host. Create your login below, then connect your mailbox.</p>"
    } else {
        ""
    };
    let autocomplete = if ready == 0 {
        "new-password"
    } else {
        "current-password"
    };
    Ok(Html(format!(r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title><link rel="icon" href="/favicon.ico"><style>body{{font:16px system-ui;background:#f5f5f7;color:#202124;margin:0;display:grid;min-height:100dvh;place-items:center}}main{{width:min(340px,80vw);padding:36px;background:white;border-radius:20px;box-shadow:0 8px 40px #0001}}img{{width:64px;height:64px}}h1{{font-size:26px}}label{{display:block;margin:20px 0}}input{{display:block;box-sizing:border-box;width:100%;margin-top:8px;padding:12px;border:1px solid #aaa;border-radius:8px;font:inherit}}button{{width:100%;padding:12px;background:#27272a;color:white;border:0;border-radius:8px;font:inherit;cursor:pointer}}p{{font-size:14px;line-height:1.6;color:#555}}</style><main><img src="/dakia-icon-192.png" alt="Dakia"><h1>{title}</h1><p role="status">{error}</p><form method="post" action="/auth/login">{setup}<label>Username<input name="username" required autocomplete="username" maxlength="128"></label><label>Password<input name="password" type="password" required minlength="12" maxlength="1024" autocomplete="{autocomplete}"></label><button>Continue</button></form><p>Use at least 12 characters for your password.</p></main></html>"#)).into_response())
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    code: String,
}
static HASH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub async fn submit(
    State(app): State<App>,
    headers: HeaderMap,
    Form(input): Form<Credentials>,
) -> Result<Response> {
    match process(State(app.clone()), headers, Form(input)).await {
        Ok(response) => Ok(response),
        Err(error) => {
            let message = match error.1 {
                "invalid_setup_code" => "Check the setup code on your host and try again.",
                "try_again_later" => "Too many attempts. Wait a minute, then try again.",
                "csrf_rejected" => "Open Dakia at its configured HTTPS address, then try again.",
                "setup_already_complete" => "Setup is complete. Sign in with your account.",
                "invalid_credentials" => {
                    "Check your username and password. Passwords need at least 12 characters."
                }
                _ => "Sign-in is unavailable. Check the server configuration and try again.",
            };
            if !app.config.issuer.is_empty() {
                return Err(error);
            }
            let mut response = render(&app, message).await?;
            *response.status_mut() = error.0;
            Ok(response)
        }
    }
}

async fn process(
    State(app): State<App>,
    headers: HeaderMap,
    Form(input): Form<Credentials>,
) -> Result<Response> {
    if !app.config.issuer.is_empty() {
        return Err(ApiFailure(StatusCode::NOT_FOUND, "local_login_disabled"));
    }
    if headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) != Some(app.config.origin.as_str())
    {
        return Err(ApiFailure(StatusCode::FORBIDDEN, "csrf_rejected"));
    }
    if input.username.is_empty()
        || input.username.len() > 128
        || input.password.len() < 12
        || input.password.len() > 1024
        || input.code.len() > 256
    {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_credentials"));
    }
    let now = Utc::now().timestamp();
    let attempts: i64 = sqlx::query_scalar("INSERT INTO login_throttle(id,window_start,attempts) VALUES(1,?,1) ON CONFLICT(id) DO UPDATE SET attempts=CASE WHEN window_start <= excluded.window_start-60 THEN 1 ELSE attempts+1 END, window_start=CASE WHEN window_start <= excluded.window_start-60 THEN excluded.window_start ELSE window_start END RETURNING attempts")
        .bind(now).fetch_one(app.store.control()).await.map_err(StorageError::from)?;
    if attempts > 10 {
        return Err(ApiFailure(StatusCode::TOO_MANY_REQUESTS, "try_again_later"));
    }
    let permit = HASH_SLOTS
        .try_acquire()
        .map_err(|_| ApiFailure(StatusCode::TOO_MANY_REQUESTS, "try_again_later"))?;
    let existing = sqlx::query("SELECT user_id,username,password_hash FROM local_login WHERE id=1")
        .fetch_optional(app.store.control())
        .await
        .map_err(StorageError::from)?;
    let user = if let Some(row) = existing {
        let username: String = row.get("username");
        let encoded: String = row.get("password_hash");
        let valid = tokio::task::spawn_blocking(move || {
            PasswordHash::new(&encoded)
                .map(|h| {
                    Argon2::default()
                        .verify_password(input.password.as_bytes(), &h)
                        .is_ok()
                })
                .unwrap_or(false)
        })
        .await
        .map_err(internal)?;
        if !valid || username != input.username {
            return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_credentials"));
        }
        row.get::<String, _>("user_id")
    } else {
        let expected: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='local_setup_token'")
                .fetch_optional(app.store.control())
                .await
                .map_err(StorageError::from)?;
        if expected.as_deref() != Some(hash(&input.code).as_str()) {
            return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_setup_code"));
        }
        let encoded = tokio::task::spawn_blocking(move || {
            Argon2::default()
                .hash_password(input.password.as_bytes(), &SaltString::generate(&mut OsRng))
                .map(|h| h.to_string())
                .map_err(|_| "password hashing failed")
        })
        .await
        .map_err(internal)?
        .map_err(internal)?;
        let user = uuid::Uuid::new_v4().to_string();
        let mut tx = app
            .store
            .control()
            .begin()
            .await
            .map_err(StorageError::from)?;
        let inserted = sqlx::query("INSERT INTO users(id,issuer,subject) SELECT ?,'dakia:local',? WHERE NOT EXISTS(SELECT 1 FROM users)").bind(&user).bind(&input.username).execute(&mut *tx).await.map_err(StorageError::from)?;
        if inserted.rows_affected() != 1 {
            return Err(ApiFailure(StatusCode::CONFLICT, "setup_already_complete"));
        }
        sqlx::query("INSERT INTO local_login(id,user_id,username,password_hash) VALUES(1,?,?,?)")
            .bind(&user)
            .bind(input.username)
            .bind(encoded)
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from)?;
        sqlx::query("DELETE FROM settings WHERE key='local_setup_token'")
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from)?;
        tx.commit().await.map_err(StorageError::from)?;
        user
    };
    drop(permit);
    let token = random();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
        .bind(hash(&token))
        .bind(user)
        .bind(random())
        .bind(now + 86400 * 7)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    Ok((
        [(header::SET_COOKIE, session_cookie(&token, 86400 * 7))],
        Redirect::to("/"),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    async fn fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key");
        initialize_credential_key(dir.path().to_str().unwrap(), key.to_str().unwrap()).unwrap();
        let store = Store::open(dir.path(), std::fs::read(&key).unwrap().try_into().unwrap())
            .await
            .unwrap();
        initialize_local_login(&store, key.to_str().unwrap())
            .await
            .unwrap();
        let app = App {
            store,
            config: Arc::new(Config {
                origin: "https://mail.test".into(),
                issuer: "".into(),
                client_id: "".into(),
                client_secret: "".into(),
                web_root: "/missing".into(),
            }),
            mail: Default::default(),
        };
        (dir, app)
    }
    fn request(origin: &str, body: String) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header("origin", origin)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap()
    }
    #[tokio::test]
    async fn setup_login_sessions_and_replay() {
        let (dir, app) = fixture().await;
        let code = std::fs::read_to_string(dir.path().join("setup-token")).unwrap();
        let body = format!("username=owner&password=long-test-password&code={code}");
        let router = router(app.clone());
        let form = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/auth/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(form.headers()["referrer-policy"], "same-origin");
        assert_eq!(
            router
                .clone()
                .oneshot(request("https://evil.test", body.clone()))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            router
                .clone()
                .oneshot(request(
                    "https://mail.test",
                    "username=owner&password=long-test-password&code=wrong".into()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = router
            .clone()
            .oneshot(request("https://mail.test", body.clone()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains("HttpOnly; Secure; SameSite=Lax"));
        let session = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/session")
                    .header(header::COOKIE, cookie.split(';').next().unwrap())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session.status(), StatusCode::OK);
        let stored: String = sqlx::query_scalar("SELECT password_hash FROM local_login")
            .fetch_one(app.store.control())
            .await
            .unwrap();
        assert!(stored.starts_with("$argon2id$"));
        let token: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM settings WHERE key='local_setup_token'")
                .fetch_one(app.store.control())
                .await
                .unwrap();
        assert_eq!(token, 0);
        assert_eq!(
            router
                .clone()
                .oneshot(request(
                    "https://mail.test",
                    format!("username=intruder&password=another-password&code={code}")
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            router
                .clone()
                .oneshot(request(
                    "https://mail.test",
                    "username=owner&password=wrong-password".into()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            router
                .oneshot(request(
                    "https://mail.test",
                    "username=owner&password=long-test-password".into()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::SEE_OTHER
        );
    }
    #[tokio::test]
    async fn oidc_data_cannot_be_claimed_and_configuration_stays_valid() {
        let (_dir, app) = fixture().await;
        app.store
            .user("https://issuer.test", "existing-owner")
            .await
            .unwrap();
        assert!(initialize_local_login(&app.store, "/nonexistent/key")
            .await
            .is_err());
        let mut config = (*app.config).clone();
        assert!(config.validate().is_ok());
        config.issuer = "https://issuer.test".into();
        assert!(config.validate().is_err());
        config.client_id = "client".into();
        config.client_secret = "secret".into();
        assert!(config.validate().is_ok());
        let app = App {
            config: Arc::new(config),
            ..app
        };
        assert_eq!(
            router(app)
                .oneshot(request(
                    "https://mail.test",
                    "username=owner&password=long-test-password".into()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn throttles_and_preserves_keys() {
        let (dir, app) = fixture().await;
        for _ in 0..10 {
            assert_eq!(
                router(app.clone())
                    .oneshot(request(
                        "https://mail.test",
                        "username=owner&password=long-test-password&code=wrong".into()
                    ))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            router(app)
                .oneshot(request(
                    "https://mail.test",
                    "username=owner&password=long-test-password&code=wrong".into()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let key = dir.path().join("key");
        let before = std::fs::read(&key).unwrap();
        initialize_credential_key(dir.path().to_str().unwrap(), key.to_str().unwrap()).unwrap();
        assert_eq!(std::fs::read(&key).unwrap(), before);
        std::fs::remove_file(&key).unwrap();
        assert!(
            initialize_credential_key(dir.path().to_str().unwrap(), key.to_str().unwrap()).is_err()
        );
    }
}
