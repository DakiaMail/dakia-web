//! Native Graph delta rounds are durable and never use IMAP's list-page pruning.
use crate::{
    mail::{provider_error, MailRuntime},
    ApiFailure, App,
};
use axum::http::StatusCode;
use chrono::{SecondsFormat, Utc};
use dakia_mail_provider::{self as p, GraphProvider};
use dakia_storage::{AuthorizedAccount, Error, Store};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests {
    use super::*;
    use p::MetadataLookup;
    #[tokio::test]
    async fn sent_metadata_is_fetched_once_per_folder_and_drafts_keep_full_observations() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path(), [7; 32]).await.unwrap();
        let user = store
            .user("https://fixture.invalid", "owner")
            .await
            .unwrap();
        let input=serde_json::from_value(serde_json::json!({"email":"fixture@example.test","display_name":"Fixture","provider":"imap","imap_host":"imap.test","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.test","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
        let account = store.connect(&user, &input).await.unwrap();
        let a = store.authorize(&user, &account.id).await.unwrap();
        let id = store.message_id(&a, "graph:message").await.unwrap();
        let message=serde_json::from_value(serde_json::json!({"id":id,"account_id":a.id(),"thread_id":"fixture","provider_id":"graph:message","folder_ids":["Sent"],"sender":{"email":"fixture@example.test","name":null},"to":[],"subject":"Fixture","preview":"","received_at":"2026-11-11T00:00:00Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
        store.ingest_messages(&a, vec![message], &[]).await.unwrap();
        let identity = p::Identity::Graph {
            account_id: a.id().into(),
            message_id: "message".into(),
        };
        let known = Known {
            store: &store,
            account: &a,
            folder: "Sent",
            full: false,
            sent: true,
        };
        assert!(!known.identity_known(&identity).await.unwrap());
        store
            .put_provider_document(&a, &format!("member-reference:{id}:Sent"), "{}")
            .await
            .unwrap();
        assert!(known.identity_known(&identity).await.unwrap());
        let drafts = Known {
            full: true,
            ..known
        };
        assert!(!drafts.identity_known(&identity).await.unwrap());
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Schedule {
    index: usize,
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    cursor: p::GraphCursor,
    at: i64,
    wake: i64,
}
#[derive(Serialize, Deserialize)]
struct Arrivals {
    since: String,
    until: String,
    next: Option<String>,
    at: i64,
}

struct Known<'a> {
    store: &'a Store,
    account: &'a AuthorizedAccount,
    folder: &'a str,
    full: bool,
    sent: bool,
}
#[async_trait::async_trait]
impl p::MetadataLookup for Known<'_> {
    async fn identity_known(&self, identity: &p::Identity) -> p::Result<bool> {
        let p::Identity::Graph {
            account_id,
            message_id,
        } = identity
        else {
            return Ok(false);
        };
        if account_id != self.account.id() {
            return Err(p::error(
                p::ErrorKind::InvalidInput,
                "Graph account mismatch",
            ));
        }
        if self.full {
            return Ok(false);
        }
        let known = async {
            let Some(message) = self
                .store
                .known_provider_message(self.account, &format!("graph:{message_id}"))
                .await?
            else {
                return Ok(false);
            };
            if self.sent {
                return self
                    .store
                    .provider_document(
                        self.account,
                        &format!("member-reference:{}:{}", message.id, self.folder),
                    )
                    .await
                    .map(|reference| reference.is_some());
            }
            Ok(true)
        }
        .await;
        known.map_err(|_: Error| {
            p::error(p::ErrorKind::Retryable, "Graph metadata lookup unavailable")
        })
    }
    async fn reference_known(&self, _: &p::MessageRef) -> p::Result<bool> {
        Ok(false)
    }
}

pub(crate) async fn tick(
    app: &App,
    a: &AuthorizedAccount,
    graph: &GraphProvider,
) -> crate::Result<bool> {
    crate::graph_push::maintain(app, a, graph).await?;
    let immutable = graph.immutable_ids().await.map_err(provider_error)?;
    app.store
        .put_provider_document(
            a,
            "graph:immutable-ids",
            if immutable { "true" } else { "false" },
        )
        .await?;
    if arrivals(&app.mail, &app.store, a, graph, 60).await? {
        return Ok(true);
    }
    let folders = app.mail.folders(&app.store, a).await?;
    if folders.is_empty() {
        return Ok(false);
    }
    let raw = app.store.provider_document(a, "graph:schedule").await?;
    let mut schedule: Schedule = raw
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(Error::from)?
        .unwrap_or_default();
    let wake = crate::graph_push::wake_version(&app.store, a).await?;
    for offset in 0..folders.len() {
        let index = (schedule.index + offset) % folders.len();
        let folder = &folders[index];
        let key = format!("graph:cursor:{}", folder.name);
        let raw = app.store.provider_document(a, &key).await?;
        let state: Option<Cursor> = raw
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(Error::from)?;
        if state.as_ref().is_some_and(|s| {
            !s.cursor.next && s.at > Utc::now().timestamp() - 300 && s.wake >= wake
        }) {
            continue;
        }
        let lane = app.mail.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        let _activity = app.mail.begin_sync(a).await;
        let initial = state.as_ref().is_none_or(|s| s.cursor.initial);
        if state.is_none() {
            app.store.begin_graph_scan(a, &folder.name).await?;
        }
        let known = Known {
            store: &app.store,
            account: a,
            folder: &folder.name,
            full: folder
                .attributes
                .iter()
                .any(|role| role.eq_ignore_ascii_case("Drafts")),
            sent: folder
                .attributes
                .iter()
                .any(|role| role.eq_ignore_ascii_case("Sent")),
        };
        let page = match graph
            .delta(&folder.name, state.as_ref().map(|s| &s.cursor), &known)
            .await
        {
            Ok(page) => page,
            Err(error) if error.kind == p::ErrorKind::StaleReference => {
                graph.refresh_folders().await;
                app.store.begin_graph_scan(a, &folder.name).await?;
                return Ok(true);
            }
            Err(error) => {
                if error.kind == p::ErrorKind::Authentication {
                    app.store
                        .account_state(a, dakia_domain::AccountState::Reconnect)
                        .await?;
                }
                return Err(provider_error(error));
            }
        };
        let mut rows = app
            .mail
            .ingest_metadata(&app.store, a, &folder.name, 1, page.messages)
            .await?;
        let mut updates = Vec::new();
        let mut documents = Vec::new();
        for update in page.updates {
            let p::Identity::Graph {
                account_id,
                message_id,
            } = update.identity
            else {
                return Err(ApiFailure(
                    StatusCode::BAD_GATEWAY,
                    "invalid_provider_metadata",
                ));
            };
            if account_id != a.id() {
                return Err(ApiFailure(StatusCode::FORBIDDEN, "account_mismatch"));
            }
            let mut message = app
                .store
                .known_provider_message(a, &format!("graph:{message_id}"))
                .await?
                .ok_or(Error::NotFound)?;
            message.unread = !update.seen;
            message.starred = update.starred;
            message.folder_ids = vec![folder.name.clone()];
            let raw = serde_json::to_string(&update.reference).map_err(Error::from)?;
            documents.push((format!("reference:{}", message.id), raw.clone()));
            documents.push((
                format!("member-reference:{}:{}", message.id, folder.name),
                raw,
            ));
            updates.push(message);
        }
        rows.extend(app.store.ingest_messages(a, updates, &documents).await?);
        let complete = !page.cursor.next;
        let state = Cursor {
            cursor: page.cursor,
            at: Utc::now().timestamp(),
            wake,
        };
        let cursor = serde_json::to_string(&state).map_err(Error::from)?;
        app.store
            .commit_graph_page(
                a,
                &folder.name,
                &rows,
                &page.removed,
                &cursor,
                initial,
                complete,
            )
            .await?;
        schedule.index = (index + 1) % folders.len();
        app.store
            .put_provider_document(
                a,
                "graph:schedule",
                &serde_json::to_string(&schedule).map_err(Error::from)?,
            )
            .await?;
        if folder.name.eq_ignore_ascii_case("INBOX") {
            app.store.inbox_refresh_result(a, None).await?;
        }
        return Ok(true);
    }
    Ok(false)
}

/// Called under the existing sync lane; history and arrival cursors are independent.
pub(crate) async fn arrivals(
    runtime: &MailRuntime,
    store: &Store,
    a: &AuthorizedAccount,
    graph: &GraphProvider,
    interval: i64,
) -> crate::Result<bool> {
    let now = Utc::now();
    let raw = store.provider_document(a, "graph:arrivals").await?;
    let mut state: Arrivals = raw
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(Error::from)?
        .unwrap_or(Arrivals {
            since: (now - chrono::Duration::minutes(5)).to_rfc3339_opts(SecondsFormat::Secs, true),
            until: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            next: None,
            at: 0,
        });
    let wake = crate::graph_push::wake_version(store, a).await?;
    let head_wake = store
        .provider_document(a, "graph:head-wake")
        .await?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(-1);
    if state.next.is_none() && state.at > now.timestamp() - interval && head_wake >= wake {
        return Ok(false);
    }
    if state.next.is_none() {
        state.until = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    }
    let lane = runtime.reconciliation_lane(a).await;
    let _guard = lane.lock().await;
    let _activity = runtime.begin_sync(a).await;
    let page = match graph
        .arrivals("INBOX", &state.since, &state.until, state.next.as_deref())
        .await
    {
        Ok(page) => page,
        Err(e) if e.kind == p::ErrorKind::StaleReference => {
            state.next = None;
            state.at = 0;
            graph.refresh_folders().await;
            store
                .put_provider_document(
                    a,
                    "graph:arrivals",
                    &serde_json::to_string(&state).map_err(Error::from)?,
                )
                .await?;
            return Ok(true);
        }
        Err(e) => {
            if e.kind == p::ErrorKind::Authentication {
                store
                    .account_state(a, dakia_domain::AccountState::Reconnect)
                    .await?;
            }
            return Err(provider_error(e));
        }
    };
    // Establish notification bootstrap before ingesting any historical observations.
    store.record_incremental(a, "INBOX", &[], "{}").await?;
    let rows = runtime
        .ingest_metadata(store, a, "INBOX", 1, page.messages)
        .await?;
    state.next = page.next;
    state.at = now.timestamp();
    if state.next.is_none() {
        state.since = (chrono::DateTime::parse_from_rfc3339(&state.until)
            .map_err(|_| Error::Invalid)?
            - chrono::Duration::minutes(5))
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    }
    let checkpoint = serde_json::to_string(&state).map_err(Error::from)?;
    store
        .record_incremental(a, "INBOX", &rows, &checkpoint)
        .await?;
    store
        .put_provider_document(a, "graph:arrivals", &checkpoint)
        .await?;
    store
        .put_provider_document(a, "graph:head-wake", &wake.to_string())
        .await?;
    store.inbox_refresh_result(a, None).await?;
    Ok(true)
}
