use super::*;

pub(super) async fn list(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
) -> Result<Json<Vec<MailRule>>> {
    Ok(Json(app.store.rules(&user.user).await?))
}
pub(super) async fn create(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Json(request): Json<CreateRule>,
) -> Result<Json<MailRule>> {
    Ok(Json(app.store.create_rule(&user.user, request).await?))
}
pub(super) async fn update(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(id): Path<String>,
    Json(request): Json<UpdateRule>,
) -> Result<Json<MailRule>> {
    Ok(Json(app.store.update_rule(&user.user, &id, request).await?))
}
pub(super) async fn activity(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<Vec<RuleActivity>>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.rule_activity(&a).await?))
}
