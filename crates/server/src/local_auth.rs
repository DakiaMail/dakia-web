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
    render(app, "", "", "", false).await
}
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
async fn render(
    app: &App,
    error: &str,
    username: &str,
    code: &str,
    joining: bool,
) -> Result<Response> {
    let username = escape_html(username);
    let code = escape_html(code);
    let notice = if error.is_empty() {
        String::new()
    } else {
        format!("<p role=\"alert\">{}</p>", escape_html(error))
    };
    let ready: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_login")
        .fetch_one(app.store.control())
        .await
        .map_err(StorageError::from)?;
    let title = if joining {
        "Create your Dakia account"
    } else if ready == 0 {
        "Set up Dakia"
    } else {
        "Sign in to Dakia"
    };
    let setup = if joining {
        format!("<label>Invitation code<input id=\"invite-code\" name=\"code\" value=\"{code}\" required autocomplete=\"off\" autocapitalize=\"none\" spellcheck=\"false\" maxlength=\"256\"></label><p>Create your own login. Your email accounts stay separate.</p>")
    } else if ready == 0 {
        format!("<label>Setup code<input name=\"code\" value=\"{code}\" required autocomplete=\"off\" autocapitalize=\"none\" autocorrect=\"off\" spellcheck=\"false\" maxlength=\"256\"></label><p>Read the setup-token file on your host. Create your login below, then connect your mailbox.</p>")
    } else {
        String::new()
    };
    let action = if joining { "/auth/join" } else { "/auth/login" };
    let script = if joining {
        r#"<script>const code=location.hash.slice(1)||history.state?.dakiaInvite;if(typeof code==="string"&&code.length<=256){document.getElementById("invite-code").value=code;history.replaceState({dakiaInvite:code},"",location.pathname)}</script>"#
    } else {
        ""
    };
    let autocomplete = if joining || ready == 0 {
        "new-password"
    } else {
        "current-password"
    };
    Ok(Html(format!(r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title><link rel="icon" href="/favicon.ico"><style>*{{box-sizing:border-box}}body{{font:16px system-ui;background:#f5f5f7;color:#202124;margin:0;display:grid;min-height:100dvh;place-items:center;padding:24px 16px}}main{{width:100%;max-width:412px;padding:36px;background:white;border-radius:20px;box-shadow:0 8px 40px #0001}}img{{width:64px;height:64px}}h1{{font-size:26px}}label{{display:block;margin:20px 0}}input{{display:block;box-sizing:border-box;width:100%;margin-top:8px;padding:12px;border:1px solid #aaa;border-radius:8px;font:inherit}}button{{width:100%;min-height:44px;padding:12px;background:#27272a;color:white;border:0;border-radius:8px;font:inherit;cursor:pointer}}p{{font-size:14px;line-height:1.6;color:#555}}@media(max-width:420px){{body{{padding:16px}}main{{padding:24px}}}}</style><main><img src="/dakia-icon-192.png" alt="Dakia"><h1>{title}</h1>{notice}<form method="post" action="{action}">{setup}<label>Username<input name="username" value="{username}" required autocomplete="username" autocapitalize="none" autocorrect="off" spellcheck="false" maxlength="128"></label><label>Password<input name="password" type="password" required minlength="12" maxlength="1024" autocomplete="{autocomplete}"></label><button>Continue</button></form><p>Use at least 12 characters for your password.</p></main>{script}</html>"#)).into_response())
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    code: String,
}
static HASH_SLOTS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));
static DUMMY_HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"not-a-user-password", &SaltString::generate(&mut OsRng))
        .unwrap()
        .to_string()
});
pub(super) async fn password_hash(password: String) -> Result<String> {
    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        HASH_SLOTS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| ApiFailure(StatusCode::TOO_MANY_REQUESTS, "try_again_later"))?
    .map_err(internal)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|h| h.to_string())
            .map_err(|_| "password hashing failed")
    })
    .await
    .map_err(internal)?
    .map_err(internal)
}
async fn verify_password(password: String, encoded: Option<String>) -> Result<bool> {
    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        HASH_SLOTS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| ApiFailure(StatusCode::TOO_MANY_REQUESTS, "try_again_later"))?
    .map_err(internal)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let encoded = encoded.unwrap_or_else(|| DUMMY_HASH.clone());
        PasswordHash::new(&encoded)
            .map(|h| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &h)
                    .is_ok()
            })
            .unwrap_or(false)
    })
    .await
    .map_err(internal)
}
async fn throttle(app: &App, username: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query("DELETE FROM auth_attempts WHERE window_start<?")
        .bind(now - 120)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    for (bucket, limit) in [
        ("global".to_string(), 100),
        (format!("user:{}", hash(username)), 10),
    ] {
        let attempts: i64 = sqlx::query_scalar("INSERT INTO auth_attempts(bucket,window_start,attempts) VALUES(?,?,1) ON CONFLICT(bucket) DO UPDATE SET attempts=CASE WHEN window_start <= excluded.window_start-60 THEN 1 ELSE attempts+1 END,window_start=CASE WHEN window_start <= excluded.window_start-60 THEN excluded.window_start ELSE window_start END RETURNING attempts").bind(bucket).bind(now).fetch_one(app.store.control()).await.map_err(StorageError::from)?;
        if attempts > limit {
            return Err(ApiFailure(StatusCode::TOO_MANY_REQUESTS, "try_again_later"));
        }
    }
    Ok(())
}
pub(super) async fn join_page(State(app): State<App>) -> Result<Response> {
    if !app.config.issuer.is_empty() {
        return Err(ApiFailure(StatusCode::NOT_FOUND, "local_login_disabled"));
    }
    render(&app, "", "", "", true).await
}
pub(super) async fn join_submit(
    State(app): State<App>,
    headers: HeaderMap,
    Form(input): Form<Credentials>,
) -> Result<Response> {
    submit_mode(app, headers, input, true).await
}

pub async fn submit(
    State(app): State<App>,
    headers: HeaderMap,
    Form(input): Form<Credentials>,
) -> Result<Response> {
    submit_mode(app, headers, input, false).await
}
async fn submit_mode(
    app: App,
    headers: HeaderMap,
    input: Credentials,
    joining: bool,
) -> Result<Response> {
    // Preserve non-password fields on retries, bounding reflected input before escaping.
    let username: String = input.username.chars().take(128).collect();
    let code: String = input.code.chars().take(256).collect();
    match process(State(app.clone()), headers, Form(input), joining).await {
        Ok(response) => Ok(response),
        Err(error) => {
            let message = match error.1 {
                "invalid_invitation" => {
                    "This invitation is invalid, expired, or already used. Ask for a new one."
                }
                "username_taken" => "That username is already in use. Choose another.",
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
            let mut response = render(&app, message, &username, &code, joining).await?;
            *response.status_mut() = error.0;
            Ok(response)
        }
    }
}

async fn process(
    State(app): State<App>,
    headers: HeaderMap,
    Form(input): Form<Credentials>,
    joining: bool,
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
    throttle(&app, &input.username).await?;
    let initialized: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM local_login)")
        .fetch_one(app.store.control())
        .await
        .map_err(StorageError::from)?;
    let user = if joining {
        crate::local_users::accept(&app, &input.username, input.password, &input.code).await?
    } else if initialized {
        let row = sqlx::query("SELECT user_id,password_hash FROM local_login WHERE username=?")
            .bind(&input.username)
            .fetch_optional(app.store.control())
            .await
            .map_err(StorageError::from)?;
        let valid =
            verify_password(input.password, row.as_ref().map(|r| r.get("password_hash"))).await?;
        if !valid || row.is_none() {
            return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_credentials"));
        }
        row.unwrap().get::<String, _>("user_id")
    } else {
        let expected: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key='local_setup_token'")
                .fetch_optional(app.store.control())
                .await
                .map_err(StorageError::from)?;
        if expected.as_deref() != Some(hash(&input.code).as_str()) {
            return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_setup_code"));
        }
        let encoded = password_hash(input.password).await?;
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
        sqlx::query(
            "INSERT INTO local_login(id,user_id,username,password_hash,is_owner) VALUES(1,?,?,?,1)",
        )
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
    finish_login(&app, user).await
}
pub(super) async fn finish_login(app: &App, user: String) -> Result<Response> {
    let now = Utc::now().timestamp();
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
    async fn failed_setup_retains_escaped_fields_but_never_passwords() {
        let (_dir, app) = fixture().await;
        let response = router(app)
            .oneshot(request(
                "https://mail.test",
                "username=%3Cowner%3E&password=never-echo-password&code=bad%22%3E%3Cscript%3E"
                    .into(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(html.contains("value=\"&lt;owner&gt;\""));
        assert!(html.contains("value=\"bad&quot;&gt;&lt;script&gt;\""));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("autocapitalize=\"none\""));
        assert!(!html.contains("never-echo-password"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("type=\"password\" required"));
    }

    async fn auth_json(
        app: &App,
        token: &str,
        method: &str,
        uri: &str,
        csrf: bool,
    ) -> axum::response::Response {
        let mut r = Request::builder()
            .method(method)
            .uri(uri)
            .header("cookie", token);
        if csrf {
            r = r.header("origin", "https://mail.test").header(
                "x-csrf-token",
                session_data(app, token).await["csrf_token"]
                    .as_str()
                    .unwrap(),
            );
        }
        router(app.clone())
            .oneshot(r.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }
    async fn json(response: Response) -> serde_json::Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap()
    }
    async fn session_data(app: &App, cookie: &str) -> serde_json::Value {
        let r = Request::builder()
            .uri("/api/session")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        json(router(app.clone()).oneshot(r).await.unwrap()).await
    }
    fn session_token(response: &Response) -> String {
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }
    async fn owner(dir: &tempfile::TempDir, app: &App) -> String {
        let code = std::fs::read_to_string(dir.path().join("setup-token")).unwrap();
        let response = router(app.clone())
            .oneshot(request(
                "https://mail.test",
                format!("username=owner&password=long-test-password&code={code}"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        session_token(&response)
    }
    fn join_request(username: &str, code: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/auth/join")
            .header("origin", "https://mail.test")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "username={username}&password=another-long-password&code={code}"
            )))
            .unwrap()
    }
    #[tokio::test]
    async fn invitations_are_owner_only_single_use_and_users_stay_separate() {
        let (dir, app) = fixture().await;
        let owner = owner(&dir, &app).await;
        assert_eq!(
            auth_json(&app, &owner, "POST", "/api/invitations", false)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let r = auth_json(&app, &owner, "POST", "/api/invitations", true).await;
        assert_eq!(r.status(), StatusCode::OK);
        let invitation = json(r).await;
        let code = invitation["url"]
            .as_str()
            .unwrap()
            .split('#')
            .nth(1)
            .unwrap();
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM local_invitations")
            .fetch_one(app.store.control())
            .await
            .unwrap();
        assert_ne!(stored, code);
        // A duplicate name must not consume the invitation.
        assert_eq!(
            router(app.clone())
                .oneshot(join_request("owner", code))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        let r = router(app.clone())
            .oneshot(join_request("guest", code))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        let guest = session_token(&r);
        let owner_data = session_data(&app, &owner).await;
        let guest_data = session_data(&app, &guest).await;
        assert_ne!(owner_data["user_id"], guest_data["user_id"]);
        assert_eq!(
            auth_json(&app, &guest, "POST", "/api/invitations", true)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            auth_json(&app, &guest, "GET", "/api/invitations", false)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            auth_json(
                &app,
                &guest,
                "DELETE",
                &format!("/api/invitations/{}", invitation["id"].as_str().unwrap()),
                true
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            router(app.clone())
                .oneshot(join_request("intruder", code))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let login = router(app.clone())
            .oneshot(request(
                "https://mail.test",
                "username=guest&password=another-long-password".into(),
            ))
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            session_data(&app, &session_token(&login)).await["user_id"],
            guest_data["user_id"]
        );
        assert_eq!(
            json(auth_json(&app, &guest, "GET", "/api/local-access", false).await).await
                ["can_invite"],
            false
        );
        assert_eq!(
            json(auth_json(&app, &owner, "GET", "/api/local-access", false).await).await
                ["can_invite"],
            true
        );
        // One user's failed passwords do not lock out another user.
        for _ in 0..10 {
            let _ = router(app.clone())
                .oneshot(request(
                    "https://mail.test",
                    "username=bad-actor&password=wrong-password".into(),
                ))
                .await
                .unwrap();
        }
        assert_eq!(
            router(app.clone())
                .oneshot(request(
                    "https://mail.test",
                    "username=guest&password=another-long-password".into()
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::SEE_OTHER
        );
    }
    #[tokio::test]
    async fn invitations_expire_revoke_and_do_not_survive_backup_restore() {
        let (dir, app) = fixture().await;
        let owner = owner(&dir, &app).await;
        for revoke in [false, true] {
            let invite =
                json(auth_json(&app, &owner, "POST", "/api/invitations", true).await).await;
            let code = invite["url"].as_str().unwrap().split('#').nth(1).unwrap();
            if revoke {
                assert_eq!(
                    auth_json(
                        &app,
                        &owner,
                        "DELETE",
                        &format!("/api/invitations/{}", invite["id"].as_str().unwrap()),
                        true
                    )
                    .await
                    .status(),
                    StatusCode::NO_CONTENT
                );
            } else {
                sqlx::query("UPDATE local_invitations SET expires_at=0")
                    .execute(app.store.control())
                    .await
                    .unwrap();
            }
            assert_eq!(
                router(app.clone())
                    .oneshot(join_request("not-created", code))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        let _ = auth_json(&app, &owner, "POST", "/api/invitations", true).await;
        let backup = dir.path().join("backup");
        app.store.backup(&backup).await.unwrap();
        let key: [u8; 32] = std::fs::read(dir.path().join("key"))
            .unwrap()
            .try_into()
            .unwrap();
        let restored = Store::restore(&backup, dir.path().join("restored"), key)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_invitations")
            .fetch_one(restored.control())
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert!(
            initialize_local_login(&restored, dir.path().join("key").to_str().unwrap())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn concurrent_claims_create_only_one_invited_user() {
        let (dir, app) = fixture().await;
        let owner = owner(&dir, &app).await;
        let invite = json(auth_json(&app, &owner, "POST", "/api/invitations", true).await).await;
        let code = invite["url"].as_str().unwrap().split('#').nth(1).unwrap();
        let (a, b) = tokio::join!(
            router(app.clone()).oneshot(join_request("first", code)),
            router(app.clone()).oneshot(join_request("second", code))
        );
        let mut statuses = vec![a.unwrap().status().as_u16(), b.unwrap().status().as_u16()];
        statuses.sort();
        assert_eq!(statuses, vec![303, 401]);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_login")
            .fetch_one(app.store.control())
            .await
            .unwrap();
        assert_eq!(count, 2);
    }
    #[tokio::test]
    async fn migration_preserves_original_owner_and_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sqlite");
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../storage/migrations/control/0001_initial.sql"
        ))
        .execute(&db)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../storage/migrations/control/0005_local_login.sql"
        ))
        .execute(&db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO users VALUES('stable-id','dakia:local','owner')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO local_login VALUES(1,'stable-id','owner','original-hash')")
            .execute(&db)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../storage/migrations/control/0006_local_invitations.sql"
        ))
        .execute(&db)
        .await
        .unwrap();
        let row = sqlx::query("SELECT user_id,password_hash,is_owner FROM local_login")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("user_id"), "stable-id");
        assert_eq!(row.get::<String, _>("password_hash"), "original-hash");
        assert_eq!(row.get::<i64, _>("is_owner"), 1);
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
