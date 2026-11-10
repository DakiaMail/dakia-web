//! Local invitations never grant access to another user's mailboxes.
use super::*;
use serde::Serialize;

pub(super) async fn is_owner(app: &App, user: &str) -> Result<bool> {
    if !app.config.issuer.is_empty() {
        return Ok(false);
    }
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM local_login WHERE user_id=? AND is_owner=1)")
        .bind(user)
        .fetch_one(app.store.control())
        .await
        .map_err(StorageError::from)
        .map_err(Into::into)
}
pub(super) async fn require_owner(app: &App, id: &Identity) -> Result<()> {
    if !is_owner(app, &id.user).await? {
        return Err(ApiFailure(StatusCode::FORBIDDEN, "owner_required"));
    }
    Ok(())
}
pub async fn access(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
) -> Result<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({"can_invite":is_owner(&app,&id.user).await?}),
    ))
}
#[derive(Serialize)]
pub struct Invitation {
    id: String,
    expires_at: i64,
}
pub async fn list(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
) -> Result<Json<Vec<Invitation>>> {
    require_owner(&app, &id).await?;
    let rows=sqlx::query("SELECT id,expires_at FROM local_invitations WHERE created_by=? AND expires_at>? ORDER BY expires_at").bind(id.user).bind(Utc::now().timestamp()).fetch_all(app.store.control()).await.map_err(StorageError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Invitation {
                id: r.get("id"),
                expires_at: r.get("expires_at"),
            })
            .collect(),
    ))
}
pub async fn create(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
) -> Result<Json<serde_json::Value>> {
    require_owner(&app, &id).await?;
    let now = Utc::now().timestamp();
    let token = random();
    let invite = uuid::Uuid::new_v4().to_string();
    let expiry = now + 86400;
    let mut tx = app
        .store
        .control()
        .begin()
        .await
        .map_err(StorageError::from)?;
    sqlx::query("DELETE FROM local_invitations WHERE expires_at<=?")
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from)?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM local_invitations")
        .fetch_one(&mut *tx)
        .await
        .map_err(StorageError::from)?;
    if count >= 20 {
        return Err(ApiFailure(
            StatusCode::TOO_MANY_REQUESTS,
            "invitation_limit",
        ));
    }
    sqlx::query(
        "INSERT INTO local_invitations(id,token_hash,created_by,expires_at) VALUES(?,?,?,?)",
    )
    .bind(&invite)
    .bind(hash(&token))
    .bind(id.user)
    .bind(expiry)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from)?;
    tx.commit().await.map_err(StorageError::from)?;
    // Fragment tokens are not sent in GET URLs, proxy logs or referrers.
    Ok(Json(
        serde_json::json!({"id":invite,"expires_at":expiry,"url":format!("{}/auth/join#{}",app.config.origin,token)}),
    ))
}
pub async fn revoke(
    State(app): State<App>,
    Extension(id): Extension<Identity>,
    Path(invite): Path<String>,
) -> Result<StatusCode> {
    require_owner(&app, &id).await?;
    let r = sqlx::query("DELETE FROM local_invitations WHERE id=? AND created_by=?")
        .bind(invite)
        .bind(id.user)
        .execute(app.store.control())
        .await
        .map_err(StorageError::from)?;
    if r.rows_affected() != 1 {
        return Err(ApiFailure(StatusCode::NOT_FOUND, "not_found"));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn accept(
    app: &App,
    username: &str,
    password: String,
    code: &str,
) -> Result<String> {
    if username.trim() != username || username.chars().any(char::is_control) {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_credentials"));
    }
    let digest = hash(code);
    let now = Utc::now().timestamp();
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM local_invitations WHERE token_hash=? AND expires_at>?)",
    )
    .bind(&digest)
    .bind(now)
    .fetch_one(app.store.control())
    .await
    .map_err(StorageError::from)?;
    if !exists {
        return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_invitation"));
    }
    let encoded = crate::local_auth::password_hash(password).await?;
    let mut tx = app
        .store
        .control()
        .begin()
        .await
        .map_err(StorageError::from)?;
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE issuer='dakia:local' AND subject=?)",
    )
    .bind(username)
    .fetch_one(&mut *tx)
    .await
    .map_err(StorageError::from)?;
    if taken {
        return Err(ApiFailure(StatusCode::CONFLICT, "username_taken"));
    }
    let consumed = sqlx::query("DELETE FROM local_invitations WHERE token_hash=? AND expires_at>?")
        .bind(&digest)
        .bind(Utc::now().timestamp())
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from)?;
    if consumed.rows_affected() != 1 {
        return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_invitation"));
    }
    let user = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO users(id,issuer,subject) VALUES(?,'dakia:local',?)")
        .bind(&user)
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from)?;
    sqlx::query("INSERT INTO local_login(user_id,username,password_hash,is_owner) VALUES(?,?,?,0)")
        .bind(&user)
        .bind(username)
        .bind(encoded)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from)?;
    tx.commit().await.map_err(StorageError::from)?;
    Ok(user)
}
