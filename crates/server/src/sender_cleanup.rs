use super::*;
use dakia_storage::{CleanupJob, CleanupRequest};

pub(super) async fn enqueue(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
    Json(request): Json<CleanupRequest>,
) -> Result<Json<CleanupJob>> {
    let a = app.store.authorize(&user.user, &account).await?;
    // Local transaction only. Existing scheduler owns every provider operation.
    let request_key = crate::diagnostics::correlation(&request.id);
    let targets = request.targets.len();
    match app.store.enqueue_sender_cleanup(&a, request).await {
        Ok(job) => {
            tracing::info!(target: "dakia_server::diagnostics", account_key = crate::diagnostics::correlation(a.id()), request_key, targets, stage = "cleanup_accepted", "sender cleanup stored");
            Ok(Json(job))
        }
        Err(error) => {
            let failure = ApiFailure::from(error);
            tracing::warn!(target: "dakia_server::diagnostics", account_key = crate::diagnostics::correlation(a.id()), request_key, targets, reason = failure.1, stage = "cleanup_rejected", "sender cleanup not accepted");
            Err(failure)
        }
    }
}
pub(super) async fn list(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path(account): Path<String>,
) -> Result<Json<Vec<CleanupJob>>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.sender_cleanup_jobs(&a).await?))
}
pub(super) async fn dismiss(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Path((account, id)): Path<(String, String)>,
) -> Result<StatusCode> {
    let a = app.store.authorize(&user.user, &account).await?;
    app.store.dismiss_sender_cleanup(&a, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
