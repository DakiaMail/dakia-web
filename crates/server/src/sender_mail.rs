//! Fast, read-only sender catalogue. Provider reconciliation remains separate.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Deserialize)]
pub(super) struct Request {
    cutoff: Option<String>,
    after: Option<String>,
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
    let settings = app.mail.settings(&app.store, &a).await?;
    let mut trash: Vec<String> = settings.trash_folder.into_iter().collect();
    // Use saved discovery, never make this local lookup wait for IMAP.
    if let Some(raw) = app.store.provider_document(&a, "probe").await? {
        if let Ok(probe) = serde_json::from_str::<dakia_mail_provider::Probe>(&raw) {
            trash.extend(
                probe
                    .folders
                    .into_iter()
                    .filter(|f| {
                        f.attributes
                            .iter()
                            .any(|v| v.trim_start_matches('\\').eq_ignore_ascii_case("Trash"))
                    })
                    .map(|f| f.name),
            );
        }
    }
    let (count, mut rows) = app
        .store
        .cached_sender_mail(
            &a,
            &source.sender.email,
            &cutoff,
            request.after.as_deref(),
            &trash,
        )
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
