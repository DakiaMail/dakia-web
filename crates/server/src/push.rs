use crate::{ApiFailure, App};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use dakia_storage::{AuthorizedAccount, Error};
use serde::Deserialize;
use sha2::{Digest, Sha256};
#[cfg(test)]
#[path = "push_summary_tests.rs"]
mod tests;
#[derive(Deserialize)]
pub(crate) struct Subscription {
    endpoint: String,
    keys: Keys,
}
#[derive(Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}
// Local metadata only: no body download, provider command, flag change or cache.
pub(crate) async fn summary(
    State(app): State<App>,
    Extension(user): Extension<crate::Identity>,
    Path((account, message)): Path<(String, String)>,
) -> crate::Result<Json<serde_json::Value>> {
    let a = app.store.authorize(&user.user, &account).await?;
    let message = app.store.message(&a, &message).await?;
    let account = app
        .store
        .accounts(&user.user)
        .await?
        .into_iter()
        .find(|account| account.id == a.id())
        .ok_or(Error::NotFound)?;
    fn bounded(value: &str, max: usize) -> String {
        value.chars().take(max).collect()
    }
    Ok(Json(serde_json::json!({
        "sender": bounded(message.sender.name.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(&message.sender.email), 120),
        "subject": bounded(&message.subject, 160),
        "preview": bounded(&message.preview, 240),
        "account": bounded(&account.email, 120),
    })))
}
async fn vapid() -> Result<serde_json::Value, Error> {
    let path = std::env::var("DAKIA_VAPID_FILE").map_err(|_| Error::Invalid)?;
    Ok(serde_json::from_slice(&tokio::fs::read(path).await?)?)
}
pub(crate) async fn public_key() -> crate::Result<Json<String>> {
    let value = vapid()
        .await
        .map_err(|_| ApiFailure(StatusCode::SERVICE_UNAVAILABLE, "push_not_configured"))?;
    Ok(Json(
        value["publicKey"]
            .as_str()
            .ok_or(ApiFailure(
                StatusCode::SERVICE_UNAVAILABLE,
                "push_not_configured",
            ))?
            .into(),
    ))
}
pub(crate) async fn subscribe(
    State(app): State<App>,
    Extension(user): Extension<crate::Identity>,
    Json(subscription): Json<Subscription>,
) -> crate::Result<StatusCode> {
    let invalid = || ApiFailure(StatusCode::BAD_REQUEST, "invalid_push_subscription");
    let u = url::Url::parse(&subscription.endpoint).map_err(|_| invalid())?;
    if u.scheme() != "https"
        || u.port().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || ![
            "fcm.googleapis.com",
            "jmt17.google.com",
            "updates.push.services.mozilla.com",
            "web.push.apple.com",
        ]
        .contains(&u.host_str().unwrap_or(""))
        || subscription.endpoint.len() > 4096
        || URL_SAFE_NO_PAD
            .decode(&subscription.keys.p256dh)
            .map_err(|_| invalid())?
            .len()
            != 65
        || URL_SAFE_NO_PAD
            .decode(&subscription.keys.auth)
            .map_err(|_| invalid())?
            .len()
            != 16
    {
        return Err(invalid());
    }
    let id = format!("{:x}", Sha256::digest(subscription.endpoint.as_bytes()));
    let document = serde_json::json!({"id":id,"user_id":user.user,"endpoint":subscription.endpoint,"keys":{"p256dh":subscription.keys.p256dh,"auth":subscription.keys.auth}});
    app.store
        .subscribe(&user.user, &id, &document.to_string())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn run(app: App) {
    let mut recovered = std::collections::HashSet::new();
    loop {
        if app.store.jobs_enabled().await.unwrap_or(false) {
            if let Ok(accounts) = app.store.job_accounts().await {
                for a in accounts {
                    if !recovered.contains(a.id()) {
                        if app.store.recover_push(&a).await.is_err() {
                            continue;
                        }
                        recovered.insert(a.id().to_owned());
                    }
                    let _ = tick(&app, &a).await;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
async fn tick(app: &App, a: &AuthorizedAccount) -> Result<(), Error> {
    let Some(job) = app.store.next_push(a).await? else {
        return Ok(());
    };
    deliver(app, a, job, |job| crate::models::node_worker("push", job)).await
}

async fn deliver<F, Fut>(
    app: &App,
    a: &AuthorizedAccount,
    mut job: serde_json::Value,
    send: F,
) -> Result<(), Error>
where
    F: FnOnce(serde_json::Value) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, Error>>,
{
    let id = job["subscription_id"]
        .as_str()
        .ok_or(Error::Invalid)?
        .to_owned();
    let message = job["message_id"].as_str().ok_or(Error::Invalid)?.to_owned();
    let current = app
        .store
        .subscriptions(a)
        .await?
        .into_iter()
        .find(|s| s["id"] == id);
    let Some(subscription) = current else {
        app.store.finish_push(a, &message, &id, None).await?;
        return Ok(());
    };
    job["subscription"] = subscription;
    if !app.store.push_send_allowed(a, &message, &id).await? {
        return Ok(());
    }
    let result = send(job.clone()).await;
    // A terminal job is not necessarily a delivered notification. Retain only
    // allowlisted outcome telemetry, never endpoint/key/payload/provider text.
    let outcome = match result.as_ref().ok().and_then(|r| r["status"].as_str()) {
        Some("sent") => "sent",
        Some("expired") => "expired",
        Some("retryable") => "retryable",
        Some("failed") => "failed",
        _ => "worker_error",
    };
    tracing::info!(outcome, "push delivery attempt completed");
    let retry = match result {
        Ok(result) if result["id"] == job["id"] => match result["status"].as_str() {
            Some("expired") => {
                app.store.expire_subscription(a, &id).await?;
                None
            }
            Some("retryable") => Some(
                result["retry_after_seconds"]
                    .as_u64()
                    .unwrap_or(60)
                    .min(3600) as u32,
            ),
            _ => None,
        },
        _ => Some(60),
    };
    app.store.finish_push(a, &message, &id, retry).await
}
