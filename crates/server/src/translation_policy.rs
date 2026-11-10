//! Installation-wide translation policy. Public weights never accept mail text.
use super::*;
use axum::body::Body;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Host,
    Device,
    Automatic,
}
impl Mode {
    pub(crate) fn permits_host(self) -> bool {
        self != Self::Device
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Update {
    mode: Mode,
}
#[derive(Serialize)]
pub(crate) struct Policy {
    mode: Mode,
    can_manage: bool,
}

pub(crate) async fn mode(app: &App) -> Result<Mode> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='translation_mode'")
            .fetch_optional(app.store.control())
            .await
            .map_err(StorageError::from)?;
    match value {
        None => Ok(Mode::Host),
        Some(value) => serde_json::from_str(&value).map_err(|_| {
            ApiFailure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid_translation_policy",
            )
        }),
    }
}
async fn can_manage(app: &App, user: &str) -> Result<bool> {
    // OIDC installations explicitly nominate stable Dakia user IDs, never an
    // arbitrary first visitor, email domain, or sender-controlled claim.
    if std::env::var("DAKIA_ADMIN_USERS")
        .unwrap_or_default()
        .split(',')
        .any(|id| !id.trim().is_empty() && id.trim() == user)
    {
        return Ok(true);
    }
    super::local_users::is_owner(app, user).await
}
pub(crate) async fn get_policy(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
) -> Result<Json<Policy>> {
    Ok(Json(Policy {
        mode: mode(&app).await?,
        can_manage: can_manage(&app, &id.user).await?,
    }))
}
pub(crate) async fn set_policy(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Json(update): Json<Update>,
) -> Result<Json<Policy>> {
    if !can_manage(&app, &id.user).await? {
        return Err(ApiFailure(StatusCode::FORBIDDEN, "administrator_required"));
    }
    sqlx::query("INSERT INTO settings(key,value) VALUES('translation_mode',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        .bind(serde_json::to_string(&update.mode).unwrap())
        .execute(app.store.control()).await.map_err(StorageError::from)?;
    Ok(Json(Policy {
        mode: update.mode,
        can_manage: true,
    }))
}

#[derive(Clone, Deserialize)]
struct Registry {
    models: Vec<Model>,
}
#[derive(Clone, Deserialize)]
struct Model {
    files: Files,
}
#[derive(Clone, Deserialize)]
struct Files {
    model: Artifact,
    shortlist: Artifact,
    vocabs: Vec<Artifact>,
}
#[derive(Clone, Deserialize)]
struct Artifact(String, usize, String);
fn artifact(hash: &str) -> Option<Artifact> {
    static ARTIFACTS: std::sync::OnceLock<Vec<Artifact>> = std::sync::OnceLock::new();
    ARTIFACTS
        .get_or_init(|| {
            let registry: Registry =
                serde_json::from_str(include_str!("../../../workers/translation/models.json"))
                    .expect("pinned registry");
            registry
                .models
                .into_iter()
                .flat_map(|m| {
                    let mut files = vec![m.files.model, m.files.shortlist];
                    files.extend(m.files.vocabs);
                    files
                })
                .collect()
        })
        .iter()
        .find(|a| a.2 == hash)
        .cloned()
}
fn verified(bytes: &[u8], artifact: &Artifact) -> bool {
    bytes.len() == artifact.1 && format!("{:x}", Sha256::digest(bytes)) == artifact.2
}
async fn model_bytes(
    artifact: &Artifact,
) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let root = std::env::var_os("DAKIA_MODEL_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("dakia-translation-models"));
    // Serialize cold public-weight downloads across clients and atomic writers.
    static DOWNLOAD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = DOWNLOAD.lock().await;
    let path = root.join(&artifact.2);
    if let Ok(bytes) = tokio::fs::read(&path).await {
        if verified(&bytes, artifact) {
            return Ok(bytes);
        }
    }
    if !artifact
        .0
        .starts_with("https://firefox-settings-attachments.cdn.mozilla.net/")
        || artifact.1 > 150_000_000
    {
        return Err("invalid model".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let mut response = client.get(&artifact.0).send().await?.error_for_status()?;
    let mut bytes = Vec::with_capacity(artifact.1);
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > artifact.1 {
            return Err("model too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !verified(&bytes, artifact) {
        return Err("model integrity failure".into());
    }
    tokio::fs::create_dir_all(&root).await?;
    let temporary = root.join(format!("{}.{}.part", artifact.2, uuid::Uuid::new_v4()));
    tokio::fs::write(&temporary, &bytes).await?;
    if let Err(error) = tokio::fs::rename(&temporary, &path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error.into());
    }
    Ok(bytes)
}
pub(crate) async fn download(Path(hash): Path<String>) -> Result<Response> {
    let artifact = artifact(&hash).ok_or(ApiFailure(StatusCode::NOT_FOUND, "model_not_found"))?;
    let bytes = model_bytes(&artifact)
        .await
        .map_err(|_| ApiFailure(StatusCode::BAD_GATEWAY, "model_download_failed"))?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    async fn fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), [57; 32]).await.unwrap();
        let owner = store
            .user("dakia:local", "translation-owner")
            .await
            .unwrap();
        let guest = store
            .user("dakia:local", "translation-guest")
            .await
            .unwrap();
        for (token, user) in [("policy-owner", &owner), ("policy-guest", &guest)] {
            sqlx::query("INSERT INTO sessions(token_hash,user_id,csrf,expires_at) VALUES(?,?,?,?)")
                .bind(hash(token))
                .bind(user)
                .bind("policy-csrf")
                .bind(4102444800_i64)
                .execute(store.control())
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO local_login(id,user_id,username,password_hash,is_owner) VALUES(1,?,'owner','unused',1)")
            .bind(owner).execute(store.control()).await.unwrap();
        let app = App {
            store,
            mail: crate::mail::MailRuntime::default(),
            config: Arc::new(Config {
                origin: "https://mail.test".into(),
                issuer: String::new(),
                client_id: String::new(),
                client_secret: String::new(),
                web_root: "missing-fixture-assets".into(),
            }),
        };
        (dir, app)
    }
    async fn request(app: &App, token: &str, method: &str, body: &str, csrf: bool) -> Response {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri("/api/translation/settings")
            .header(header::COOKIE, format!("__Host-dakia={token}"))
            .header(header::CONTENT_TYPE, "application/json");
        if csrf {
            request = request
                .header(header::ORIGIN, "https://mail.test")
                .header("x-csrf-token", "policy-csrf");
        }
        crate::router(app.clone())
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }
    async fn json(response: Response) -> serde_json::Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 8192)
                .await
                .unwrap(),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn admin_only_policy_persists_and_is_visible_to_other_users() {
        let (dir, app) = fixture().await;
        assert_eq!(
            json(request(&app, "policy-guest", "GET", "", false).await).await["mode"],
            "host"
        );
        assert_eq!(
            request(&app, "policy-guest", "PUT", r#"{"mode":"device"}"#, true)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(&app, "policy-owner", "PUT", r#"{"mode":"device"}"#, false)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let response = request(&app, "policy-owner", "PUT", r#"{"mode":"device"}"#, true).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await["can_manage"], true);
        assert!(!mode(&app).await.unwrap().permits_host());
        let view = json(request(&app, "policy-guest", "GET", "", false).await).await;
        assert_eq!(view["mode"], "device");
        assert_eq!(view["can_manage"], false);
        let reopened = App {
            store: Store::open(dir.path(), [57; 32]).await.unwrap(),
            ..app.clone()
        };
        assert_eq!(mode(&reopened).await.unwrap(), Mode::Device);
        assert_eq!(
            request(&app, "policy-owner", "PUT", r#"{"mode":"automatic"}"#, true)
                .await
                .status(),
            StatusCode::OK
        );
        assert!(mode(&app).await.unwrap().permits_host());
    }
    #[tokio::test]
    async fn invalid_policy_and_unknown_fields_are_rejected_and_corruption_fails_closed() {
        let (_dir, app) = fixture().await;
        for body in [
            r#"{"mode":"cloud"}"#,
            r#"{"mode":"host","can_manage":true}"#,
        ] {
            assert_eq!(
                request(&app, "policy-owner", "PUT", body, true)
                    .await
                    .status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        sqlx::query("INSERT INTO settings(key,value) VALUES('translation_mode','invalid')")
            .execute(app.store.control())
            .await
            .unwrap();
        assert!(mode(&app).await.is_err());
        assert_eq!(
            request(&app, "policy-owner", "GET", "", false)
                .await
                .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
    #[tokio::test]
    async fn anonymous_and_unregistered_model_requests_are_rejected() {
        let (_dir, app) = fixture().await;
        assert_eq!(
            request(&app, "unknown", "GET", "", false).await.status(),
            StatusCode::UNAUTHORIZED
        );
        let request = axum::http::Request::builder()
            .uri("/api/translation/models/not-a-model")
            .header(header::COOKIE, "__Host-dakia=policy-owner")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            crate::router(app).oneshot(request).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        let a = Artifact("pinned".into(), 3, format!("{:x}", Sha256::digest(b"abc")));
        assert!(verified(b"abc", &a));
        assert!(!verified(b"abd", &a));
        assert!(!verified(b"ab", &a));
    }
}
