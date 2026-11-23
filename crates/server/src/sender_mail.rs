//! Fast, read-only sender catalogue. Provider reconciliation remains separate.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
pub(super) struct Request {
    cutoff: Option<String>,
    after: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct CleanupRequest {
    sender: String,
    #[serde(flatten)]
    page: Request,
}
#[derive(Serialize)]
pub(super) struct Page {
    rows: Vec<Message>,
    count: i64,
    cutoff: String,
    after: Option<String>,
}
pub(super) async fn cached(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, message)): Path<(String, String)>,
    Query(request): Query<Request>,
) -> Result<Json<Page>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let source = app.store.message(&a, &message).await?;
    page(&app, &a, &source.sender.email, request).await
}

pub(super) async fn cleanup(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Query(request): Query<CleanupRequest>,
) -> Result<Json<Page>> {
    let a = app.store.authorize(&user.user, &account).await?;
    page(&app, &a, &request.sender, request.page).await
}

async fn page(
    app: &App,
    a: &dakia_storage::AuthorizedAccount,
    sender: &str,
    request: Request,
) -> Result<Json<Page>> {
    let cutoff = match request.cutoff {
        Some(raw) => chrono::DateTime::parse_from_rfc3339(&raw)
            .map_err(|_| ApiFailure(StatusCode::BAD_REQUEST, "invalid_cutoff"))?
            .with_timezone(&Utc),
        None => Utc::now(),
    };
    if cutoff > Utc::now() || request.after.as_ref().is_some_and(|v| v.len() > 512) {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_cutoff"));
    }
    let cutoff = cutoff.to_rfc3339();
    let settings = app.mail.settings(&app.store, a).await?;
    // Read saved folder discovery so the catalogue works without a provider connection.
    let folders = app
        .store
        .provider_document(a, "probe")
        .await?
        .and_then(|raw| serde_json::from_str::<dakia_mail_provider::Probe>(&raw).ok())
        .map(|probe| probe.folders)
        .unwrap_or_default();
    let mut excluded = crate::mail::search_exclusions(&folders, &settings);
    excluded.extend(app.store.search_excluded_folders(a).await?);
    excluded.extend(["Spam".into(), "Junk".into()]);
    let (count, mut rows) = app
        .store
        .cached_sender_mail(a, sender, &cutoff, request.after.as_deref(), &excluded)
        .await?;
    let after = if rows.len() > 100 {
        rows.truncate(100);
        rows.last().map(|m| m.id.clone())
    } else {
        None
    };
    Ok(Json(Page {
        rows,
        count,
        cutoff,
        after,
    }))
}
