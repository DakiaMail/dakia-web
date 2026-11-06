//! Bounded round-robin metadata backfill. Polling is the portable fallback.
use crate::App;
use dakia_mail_provider as p;
use dakia_storage::AuthorizedAccount;
use serde::{Deserialize, Serialize};
pub(crate) mod full_text;
pub(crate) mod starred;
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct State {
    pub(crate) folders: Vec<p::Folder>,
    index: usize,
    pub(crate) refreshed: i64,
    #[serde(default)]
    backfill_turn: bool,
    #[serde(default)]
    head_turn: bool,
    #[serde(default)]
    unfinished_turns: u8,
}
// Alternate priority after each attempted page, including errors. INBOX gets
// at least every other background turn, and other folders cannot be starved by
// a large or failing inbox. Request refreshes share the account lane, not this
// durable budget. Still at most one provider page per background turn.
fn scan_order(folders: &[p::Folder], start: usize, backfill_turn: bool) -> Vec<usize> {
    if folders.is_empty() {
        return Vec::new();
    }
    let inbox = folders
        .iter()
        .position(|f| f.name.eq_ignore_ascii_case("INBOX"));
    let others = (0..folders.len())
        .map(|i| (start + i) % folders.len())
        .filter(|i| Some(*i) != inbox);
    if backfill_turn {
        others.chain(inbox).collect()
    } else {
        inbox.into_iter().chain(others).collect()
    }
}
// Only successful provider work permits a bounded follow-up executor turn.
#[tracing::instrument(target = "dakia_server::diagnostics", skip_all, fields(account_key = crate::diagnostics::correlation(a.id()), purpose = "metadata"))]
pub(crate) async fn tick(app: &App, a: &AuthorizedAccount) -> crate::Result<bool> {
    let lane = app.mail.sync_lane(a).await;
    let _guard = lane.lock().await;
    let mut state: State = app
        .store
        .provider_document(a, "sync:folders")
        .await?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if state.folders.is_empty() || chrono::Utc::now().timestamp() - state.refreshed > 300 {
        state.folders = app.mail.folders(&app.store, a).await?;
        state.folders.sort_by_key(|f| {
            if f.name.eq_ignore_ascii_case("INBOX") {
                0
            } else if f
                .attributes
                .iter()
                .any(|s| s.trim_start_matches('\\').eq_ignore_ascii_case("Sent"))
            {
                1
            } else {
                2
            }
        });
        state.refreshed = chrono::Utc::now().timestamp();
    }
    if state.folders.is_empty() {
        return Ok(false);
    }
    // Do not restart already traversed folders while another folder has not
    // reached its end. scan_state rolls expired generations forward, so this
    // eligibility check must be read-only and precede that call. Missing folder
    // states are unfinished too. Coverage/pruning authority stays unchanged.
    let terminal = app.store.terminal_scan_folders(a).await?;
    let unfinished = state.folders.iter().any(|f| !terminal.contains(&f.name));
    // A permanently failing folder must not suppress mutable reconciliation
    // elsewhere forever. Reserve one normal round-robin turn per 16 attempts.
    let defer_restarts = unfinished && state.unfinished_turns < 16;
    let mut progressed = false;
    for index in scan_order(&state.folders, state.index, state.backfill_turn) {
        let folder = state.folders[index].name.clone();
        if folder.eq_ignore_ascii_case("INBOX") {
            app.store.seed_inbox_memberships(a).await?;
            if state.head_turn {
                // Alternate head/history within INBOX's budget. A failed head
                // consumes its turn too, never starving other-folder progress.
                let result = app.mail.poll_head(&app.store, a, &folder).await;
                if !matches!(result, Ok(false)) {
                    state.head_turn = false;
                    state.backfill_turn = true;
                    match result {
                        Ok(_) => {
                            app.store.inbox_refresh_result(a, None).await?;
                            progressed = true;
                        }
                        Err(error) => app.store.inbox_refresh_result(a, Some(error.1)).await?,
                    }
                    break;
                }
            }
        }
        if defer_restarts && terminal.contains(&folder) {
            if folder.eq_ignore_ascii_case("INBOX") {
                // Arrival heads retain their independent due-time budget even
                // when historical INBOX restarts yield to other folders.
                state.head_turn = true;
            }
            continue;
        }
        let (scan, verified) = match app.mail.prepare_scan(&app.store, a, &folder).await {
            Ok(result) => result,
            Err(error) => {
                // A failed cheap check must consume its turn too, rather than
                // pinning the scheduler to a single failing folder.
                state.index = (index + 1) % state.folders.len();
                state.backfill_turn = folder.eq_ignore_ascii_case("INBOX");
                state.head_turn |= folder.eq_ignore_ascii_case("INBOX");
                state.unfinished_turns = if defer_restarts {
                    state.unfinished_turns + 1
                } else {
                    0
                };
                if folder.eq_ignore_ascii_case("INBOX") {
                    app.store.inbox_refresh_result(a, Some(error.1)).await?;
                }
                app.store
                    .put_provider_document(a, "sync:error", error.1)
                    .await?;
                break;
            }
        };
        if verified {
            state.index = (index + 1) % state.folders.len();
            state.backfill_turn = folder.eq_ignore_ascii_case("INBOX");
            state.head_turn |= folder.eq_ignore_ascii_case("INBOX");
            state.unfinished_turns = 0;
            progressed = true;
            if folder.eq_ignore_ascii_case("INBOX") {
                app.store.inbox_refresh_result(a, None).await?;
            }
            break;
        }
        let Some((generation, continuation)) = scan else {
            if folder.eq_ignore_ascii_case("INBOX") {
                state.head_turn = true;
            }
            continue;
        };
        if !folder.eq_ignore_ascii_case("INBOX") || index == state.index % state.folders.len() {
            state.index = (index + 1) % state.folders.len();
        }
        state.backfill_turn = folder.eq_ignore_ascii_case("INBOX");
        state.unfinished_turns = if defer_restarts {
            state.unfinished_turns + 1
        } else {
            0
        };
        if folder.eq_ignore_ascii_case("INBOX") {
            state.head_turn = true;
        }
        match app
            .mail
            .record_snapshot_page(&app.store, a, &folder, generation, continuation.as_deref())
            .await
        {
            Ok(()) => {
                progressed = true;
                if folder.eq_ignore_ascii_case("INBOX") {
                    app.store.inbox_refresh_result(a, None).await?;
                }
            }
            Err(error) => {
                if folder.eq_ignore_ascii_case("INBOX") {
                    app.store.inbox_refresh_result(a, Some(error.1)).await?;
                }
                if error.1 == "provider_reference_changed" {
                    app.store.reset_scan(a, &folder).await?;
                }
                app.store
                    .put_provider_document(a, "sync:error", error.1)
                    .await?;
            }
        }
        break;
    }
    app.store
        .put_provider_document(
            a,
            "sync:folders",
            &serde_json::to_string(&state).map_err(dakia_storage::Error::from)?,
        )
        .await?;
    Ok(progressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(names: &[&str]) -> Vec<p::Folder> {
        names
            .iter()
            .map(|name| p::Folder {
                name: (*name).into(),
                attributes: Vec::new(),
                selectable: true,
            })
            .collect()
    }

    #[test]
    fn inbox_precedes_backfill_without_duplicate_or_lost_folders() {
        let f = folders(&["Archive", "inbox", "Sent", "Trash"]);
        assert_eq!(scan_order(&f, 3, false), vec![1, 3, 0, 2]);
        assert_eq!(scan_order(&f, 2, false), vec![1, 2, 3, 0]);
        assert_eq!(scan_order(&f, 3, true), vec![3, 0, 2, 1]);
    }

    #[test]
    fn absent_inbox_keeps_round_robin_and_empty_list_is_safe() {
        assert_eq!(
            scan_order(&folders(&["Archive", "Sent"]), 1, false),
            vec![1, 0]
        );
        assert!(scan_order(&[], 7, true).is_empty());
    }
}
