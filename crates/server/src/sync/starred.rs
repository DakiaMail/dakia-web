//! Independent starred discovery and body warming. No provider mutations.
use super::*;
use std::collections::BTreeMap;

#[derive(Default, Serialize, Deserialize)]
struct Scan {
    continuation: Option<String>,
    due: i64,
}
#[derive(Default, Serialize, Deserialize)]
struct StarredState {
    scans: BTreeMap<String, Scan>,
    index: usize,
    body_after: String,
    body_due: i64,
    error: Option<String>,
}

pub(crate) async fn coverage(
    app: &App,
    a: &AuthorizedAccount,
) -> crate::Result<dakia_domain::AccountCoverage> {
    let state = app
        .store
        .provider_document(a, "sync:starred")
        .await?
        .and_then(|s| serde_json::from_str::<StarredState>(&s).ok());
    let error = state.as_ref().and_then(|s| s.error.clone());
    Ok(dakia_domain::AccountCoverage {
        account_id: a.id().into(),
        folder_ids: state
            .as_ref()
            .map(|s| s.scans.keys().cloned().collect())
            .unwrap_or_default(),
        inbox: None,
        state: if error.is_some() {
            dakia_domain::CoverageState::Error
        } else if state.is_none() {
            dakia_domain::CoverageState::Pending
        } else {
            dakia_domain::CoverageState::Partial
        },
        error,
    })
}

/// One filtered provider page and at most two content reads per account/turn.
/// Separate cursors must never be passed to record_scan: a FLAGGED-only result
/// cannot certify folder coverage or prune unstarred memberships.
#[tracing::instrument(target = "dakia_server::diagnostics", skip_all, fields(account_key = crate::diagnostics::correlation(a.id()), purpose = "starred"))]
pub(crate) async fn tick(app: &App, a: &AuthorizedAccount) -> crate::Result<()> {
    let lane = app.mail.sync_lane(a).await;
    let _guard = lane.lock().await;
    let mut state: StarredState = app
        .store
        .provider_document(a, "sync:starred")
        .await?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    // Ordinary sync owns discovery and its cooldown. Reuse its last successful
    // folder list even during an outage, without adding SMTP/probe requests.
    let folders: Vec<p::Folder> = app
        .store
        .provider_document(a, "sync:folders")
        .await?
        .and_then(|s| serde_json::from_str::<super::State>(&s).ok())
        .map(|s| s.folders)
        .unwrap_or_default();
    state
        .scans
        .retain(|name, _| folders.iter().any(|f| &f.name == name));
    let start = state.index;
    for offset in 0..folders.len() {
        let index = (start + offset) % folders.len();
        let folder = &folders[index].name;
        let scan = state.scans.entry(folder.clone()).or_default();
        if scan.due > now {
            continue;
        }
        state.index = (index + 1) % folders.len();
        if scan.continuation.is_none() && app.store.verified_snapshot_current(a, folder).await? {
            scan.due = now + 300;
            state.error = None;
            tracing::info!(target: "dakia_server::diagnostics", folder_key = crate::diagnostics::correlation(folder), operation = "starred_scan", outcome = "skipped", reason = "verified_snapshot", "mail_work");
            continue;
        }

        let query = p::Search {
            starred: Some(true),
            ..Default::default()
        };
        match app
            .mail
            .page(&app.store, a, folder, query, scan.continuation.as_deref())
            .await
        {
            Ok(page) => {
                scan.continuation = page.continuation;
                scan.due = if scan.continuation.is_none() {
                    now + 300
                } else {
                    0
                };
                state.error = None;
            }
            Err(error) => {
                if error.1 == "provider_reference_changed" {
                    scan.continuation = None;
                }
                scan.due = now + 300;
                state.error = Some(error.1.into());
            }
        }
        break;
    }
    // A keyset walk includes cached entries: content() repairs missing and old
    // cache formats, overlays current flags, and avoids redundant provider I/O.
    // Advance after failures too so one oversized/offline message cannot starve
    // other stars. Retry on the next pass; cache eviction is never a flag change.
    if state.body_due <= now {
        let rows = app.store.starred_cache_rows(a, &state.body_after).await?;
        if rows.is_empty() {
            state.body_after.clear();
            state.body_due = now + 300;
        }
        for message in rows {
            state.body_after = message.id.clone();
            if let Err(error) = app.mail.content(&app.store, a, &message.id).await {
                state.error = Some(error.1.into());
            }
        }
    }
    app.store
        .put_provider_document(
            a,
            "sync:starred",
            &serde_json::to_string(&state).map_err(dakia_storage::Error::from)?,
        )
        .await?;
    Ok(())
}
