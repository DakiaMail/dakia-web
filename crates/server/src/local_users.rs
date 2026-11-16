//! Local invitations never grant access to another user's mailboxes.
use super::*;
use serde::Serialize;

pub(super) async fn is_owner(app: &App, user: &str) -> Result<bool> {
    if !app.config.issuer.is_empty() {
        return Ok(false);
    }
    Ok(app.store.control_repository().is_owner(user).await?)
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
    let rows = app
        .store
        .control_repository()
        .invitations(&id.user, Utc::now().timestamp())
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| Invitation {
                id: r.id,
                expires_at: r.expires_at,
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
    if !app
        .store
        .control_repository()
        .create_invitation(&invite, &hash(&token), &id.user, now, expiry)
        .await?
    {
        return Err(ApiFailure(
            StatusCode::TOO_MANY_REQUESTS,
            "invitation_limit",
        ));
    }
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
    if !app
        .store
        .control_repository()
        .revoke_invitation(&invite, &id.user)
        .await?
    {
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
    let exists = app
        .store
        .control_repository()
        .invitation_valid(&digest, now)
        .await?;
    if !exists {
        return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_invitation"));
    }
    let encoded = crate::local_auth::password_hash(password).await?;
    let user = uuid::Uuid::new_v4().to_string();
    match app
        .store
        .control_repository()
        .accept_invitation(&digest, Utc::now().timestamp(), &user, username, &encoded)
        .await?
    {
        dakia_storage::control_repository::InvitationClaim::Created => {}
        dakia_storage::control_repository::InvitationClaim::UsernameTaken => {
            return Err(ApiFailure(StatusCode::CONFLICT, "username_taken"))
        }
        dakia_storage::control_repository::InvitationClaim::InvalidInvitation => {
            return Err(ApiFailure(StatusCode::UNAUTHORIZED, "invalid_invitation"))
        }
    }
    Ok(user)
}
