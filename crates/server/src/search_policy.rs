//! Installation-wide semantic search policy. Keyword indexing is independent.
use super::*;
use crate::search_resources::Resources;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Automatic,
    On,
    Off,
}
impl Mode {
    fn enabled(self, resources: Resources) -> bool {
        match self {
            Self::Automatic => resources.permits_semantic(),
            Self::On => true,
            Self::Off => false,
        }
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
    enabled: bool,
    can_manage: bool,
    resources: Resources,
}
async fn mode(store: &Store) -> dakia_storage::Result<Mode> {
    match store
        .control_repository()
        .setting("semantic_search_mode")
        .await?
    {
        None => Ok(Mode::Automatic),
        Some(value) => serde_json::from_str(&value).map_err(|_| dakia_storage::Error::Invalid),
    }
}
pub async fn initialize(store: &Store) -> dakia_storage::Result<bool> {
    let enabled = mode(store).await?.enabled(Resources::detect());
    store.set_search_embeddings_enabled(enabled);
    Ok(enabled)
}
pub(crate) async fn get_policy(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
) -> Result<Json<Policy>> {
    let _guard = app.mail.search_policy_lock.lock().await;
    Ok(Json(Policy {
        mode: mode(&app.store).await?,
        enabled: app.store.search_embeddings_enabled(),
        can_manage: translation_policy::can_manage(&app, &id.user).await?,
        resources: Resources::detect(),
    }))
}
pub(crate) async fn set_policy(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Json(update): Json<Update>,
) -> Result<Json<Policy>> {
    if !translation_policy::can_manage(&app, &id.user).await? {
        return Err(ApiFailure(StatusCode::FORBIDDEN, "administrator_required"));
    }
    let _guard = app.mail.search_policy_lock.lock().await;
    let resources = Resources::detect();
    let enabled = update.mode.enabled(resources);
    app.store
        .control_repository()
        .put_setting(
            "semantic_search_mode",
            &serde_json::to_string(&update.mode).unwrap(),
        )
        .await?;
    app.store.set_search_embeddings_enabled(enabled);
    app.mail.search_queries.set_enabled(enabled);
    Ok(Json(Policy {
        mode: update.mode,
        enabled,
        can_manage: true,
        resources,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn overrides_persist_and_are_shared_by_workers() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), [84; 32]).await.unwrap();
        let worker = store.clone();
        for (mode, expected) in [(Mode::Off, false), (Mode::On, true)] {
            store
                .control_repository()
                .put_setting(
                    "semantic_search_mode",
                    &serde_json::to_string(&mode).unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(initialize(&store).await.unwrap(), expected);
            assert_eq!(worker.search_embeddings_enabled(), expected);
        }
        assert!(Mode::On.enabled(Resources {
            memory_bytes: None,
            cpus: None
        }));
        assert!(!Mode::Off.enabled(Resources {
            memory_bytes: Some(u64::MAX),
            cpus: Some(64.)
        }));
    }
    #[tokio::test]
    async fn settings_authorization_validation_and_restart() {
        use tower::ServiceExt;
        let (dir, app) = crate::translation_policy::tests::fixture().await;
        for (token, csrf, body, expected) in [
            (
                "unknown",
                true,
                r#"{"mode":"off"}"#,
                StatusCode::UNAUTHORIZED,
            ),
            (
                "policy-guest",
                true,
                r#"{"mode":"off"}"#,
                StatusCode::FORBIDDEN,
            ),
            (
                "policy-owner",
                false,
                r#"{"mode":"off"}"#,
                StatusCode::FORBIDDEN,
            ),
            (
                "policy-owner",
                true,
                r#"{"mode":"cloud"}"#,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                "policy-owner",
                true,
                r#"{"mode":"on","enabled":true}"#,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("policy-owner", true, r#"{"mode":"off"}"#, StatusCode::OK),
        ] {
            let mut req = axum::http::Request::builder()
                .method("PUT")
                .uri("/api/search/settings")
                .header(header::COOKIE, format!("__Host-dakia={token}"))
                .header(header::CONTENT_TYPE, "application/json");
            if csrf {
                req = req
                    .header(header::ORIGIN, "https://mail.test")
                    .header("x-csrf-token", "policy-csrf");
            }
            let response = crate::router(app.clone())
                .oneshot(req.body(axum::body::Body::from(body)).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{token} {body}");
        }
        assert!(!app.store.search_embeddings_enabled());
        let reopened = Store::open(dir.path(), [57; 32]).await.unwrap();
        assert!(!initialize(&reopened).await.unwrap());
        let view = get_policy(
            State(app),
            Extension(Identity {
                user: "guest".into(),
                csrf: "".into(),
                token_hash: "".into(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(view.mode, Mode::Off);
        assert!(!view.enabled);
        assert!(!view.can_manage);
    }
}
