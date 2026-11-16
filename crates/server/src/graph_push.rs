//! Content-free, secret-bound Graph notifications wake durable delta sync.
use crate::{hash, random, ApiFailure, App};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{SecondsFormat, Utc};
use dakia_mail_provider::{ErrorKind, GraphProvider};
use dakia_storage::{AuthorizedAccount, Error, Store};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Validation {
    #[serde(rename = "validationToken")]
    token: Option<String>,
}
#[derive(Deserialize)]
pub(crate) struct Notifications {
    value: Vec<Notification>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notification {
    subscription_id: String,
    client_state: Option<String>,
    lifecycle_event: Option<String>,
}

pub(crate) async fn notify(
    State(app): State<App>,
    Query(query): Query<Validation>,
    body: axum::body::Bytes,
) -> crate::Result<Response> {
    if let Some(token) = query.token {
        if token.len() > 4096 || token.contains(['\r', '\n']) {
            return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_input"));
        }
        return Ok((
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            token,
        )
            .into_response());
    }
    let events: Notifications = serde_json::from_slice(&body)
        .map_err(|_| ApiFailure(StatusCode::BAD_REQUEST, "invalid_input"))?;
    if events.value.len() > 100 {
        return Err(ApiFailure(StatusCode::BAD_REQUEST, "invalid_input"));
    }
    for event in events.value {
        let Some(secret) = event.client_state.filter(|s| s.len() <= 128) else {
            continue;
        };
        let renew = event
            .lifecycle_event
            .as_deref()
            .is_some_and(|s| matches!(s, "subscriptionRemoved" | "reauthorizationRequired"));
        let expires = if event.lifecycle_event.as_deref() == Some("reauthorizationRequired") {
            Utc::now() + chrono::Duration::minutes(1)
        } else {
            Utc::now()
        };
        let account = app
            .store
            .control_repository()
            .wake_microsoft_subscription(
                renew,
                &expires.to_rfc3339_opts(SecondsFormat::Secs, true),
                &event.subscription_id,
                &hash(&secret),
                &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            )
            .await?;
        if let Some(account) = account {
            app.mail.wake_graph(&account).await;
        }
    }
    Ok(StatusCode::ACCEPTED.into_response())
}
pub(crate) async fn wake_version(store: &Store, a: &AuthorizedAccount) -> Result<i64, Error> {
    store.authorize(a.user_id(), a.id()).await?;
    store
        .control_repository()
        .microsoft_wake_version(a.id())
        .await
}

fn public_origin(origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(host)) => {
            host.contains('.')
                && ![".localhost", ".local", ".test", ".invalid"]
                    .iter()
                    .any(|suffix| host.ends_with(suffix))
        }
        Some(url::Host::Ipv4(ip)) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !(ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
        }
        Some(url::Host::Ipv6(ip)) => {
            !ip.is_loopback()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
        }
        None => false,
    }
}
pub(crate) async fn maintain(
    app: &App,
    a: &AuthorizedAccount,
    graph: &GraphProvider,
) -> crate::Result<()> {
    if !public_origin(&app.config.origin) {
        return Ok(());
    }
    let now = Utc::now();
    let row = app
        .store
        .control_repository()
        .microsoft_subscription(a.id())
        .await?;
    if let Some(row) = &row {
        let expires = chrono::DateTime::parse_from_rfc3339(&row.expires_at).ok();
        if row.next_attempt > now.timestamp()
            || expires.is_some_and(|at| at > now + chrono::Duration::hours(12))
        {
            return Ok(());
        }
    }
    let secret = random();
    let existing: Option<String> = row.as_ref().and_then(|r| r.id.clone());
    let live = row.as_ref().is_some_and(|r| {
        chrono::DateTime::parse_from_rfc3339(&r.expires_at).is_ok_and(|at| at > now)
    });
    // Back off registration failures without claiming push is available.
    app.store
        .control_repository()
        .backoff_microsoft_subscription(
            a.id(),
            &hash(&secret),
            &now.to_rfc3339_opts(SecondsFormat::Secs, true),
            now.timestamp() + 3600,
        )
        .await?;
    let notification_url = format!("{}/api/microsoft/notifications", app.config.origin);
    let result = graph
        .subscription(
            &notification_url,
            &secret,
            existing.as_deref().filter(|_| live),
        )
        .await;
    match result {
        Ok(subscription) => {
            if existing.is_some() && !live {
                if let Some(id) = &existing {
                    let _ = graph.unsubscribe(id).await;
                }
            }
            app.store
                .control_repository()
                .save_microsoft_subscription(
                    a.id(),
                    &subscription.id,
                    existing.is_some() && live,
                    &hash(&secret),
                    &subscription.expiration_date_time,
                )
                .await?;
        }
        Err(error) if error.kind == ErrorKind::StaleReference => {
            app.store
                .control_repository()
                .expire_microsoft_subscription(
                    a.id(),
                    &now.to_rfc3339_opts(SecondsFormat::Secs, true),
                )
                .await?;
        }
        Err(_) => {} // Polling and the last valid subscription remain intact.
    }
    Ok(())
}
pub(crate) async fn prepare_removal(
    store: &Store,
    a: &AuthorizedAccount,
    graph: &GraphProvider,
) -> Option<reqwest::RequestBuilder> {
    if let Ok(Some(row)) = store
        .control_repository()
        .microsoft_subscription(a.id())
        .await
    {
        if let Some(id) = row.id {
            return graph.prepare_unsubscribe(&id).await.ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_push_requires_a_public_https_origin_but_accepts_public_ip_certificates() {
        for origin in [
            "https://mail.example.com",
            "https://8.8.8.8",
            "https://[2606:4700:4700::1111]",
        ] {
            assert!(public_origin(origin), "{origin}");
        }
        for origin in [
            "http://mail.example.com",
            "https://localhost",
            "https://mail.test",
            "https://mail.local",
            "https://127.0.0.1",
            "https://10.0.0.1",
            "https://192.168.1.1",
            "https://100.64.0.1",
            "https://[::1]",
            "https://[fd00::1]",
        ] {
            assert!(!public_origin(origin), "{origin}");
        }
    }
}
