//! Durable submission executor. SMTP delivery and Sent filing are separate phases.
#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
use crate::{mail::configuration, App};
use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use dakia_domain::*;
use dakia_mail_provider::{
    self as p,
    mime::{self, Compose, ComposeAttachment, PreparedMessage},
    MailProvider, SubmissionGate,
};
use dakia_storage::{AuthorizedAccount, PersistedMime, Store};
use std::time::Duration;

struct Gate<'a> {
    store: &'a Store,
    account: &'a AuthorizedAccount,
    id: &'a str,
}
#[async_trait]
impl SubmissionGate for Gate<'_> {
    async fn begin_data(&self) -> p::Result<()> {
        self.store
            .transition(
                self.account,
                self.id,
                SubmissionState::Claimed,
                SubmissionState::Submitting,
            )
            .await
            .map(|_| ())
            .map_err(|_| p::ProviderError {
                kind: p::ErrorKind::Retryable,
                message: "submission transition rejected".into(),
            })
    }
}
async fn prepare(
    store: &Store,
    a: &AuthorizedAccount,
    s: &Submission,
) -> dakia_storage::Result<PreparedMessage> {
    let config = configuration(a, &store.credential(a).await?);
    let persisted = if let Some(mime) = store.mime(a, &s.id).await? {
        mime
    } else {
        let draft = store.submission_snapshot(a, &s.id).await?;
        let mut attachments = Vec::new();
        for attachment in draft.attachments {
            let (metadata, bytes) = store.attachment(a, &attachment.id).await?;
            attachments.push(ComposeAttachment {
                filename: metadata.filename,
                content_type: metadata.content_type,
                bytes,
            });
        }
        let message_id = format!("<{}@dakia.local>", s.id);
        let prepared = mime::assemble(
            &config,
            Compose {
                to: draft.to.into_iter().map(|a| a.email).collect(),
                cc: draft.cc.into_iter().map(|a| a.email).collect(),
                bcc: draft.bcc.into_iter().map(|a| a.email).collect(),
                subject: draft.subject,
                text: draft.text,
                html: Some(draft.html),
                message_id: message_id.clone(),
                in_reply_to: draft.in_reply_to,
                references: (!draft.references.is_empty()).then(|| draft.references.join(" ")),
                attachments,
            },
            30 * 1024 * 1024,
        )
        .map_err(|_| dakia_storage::Error::Invalid)?;
        store
            .persist_mime(
                a,
                &s.id,
                PersistedMime {
                    raw: prepared.raw().to_vec(),
                    recipients: prepared.recipients(),
                    message_id,
                },
            )
            .await?
    };
    PreparedMessage::restore(
        &config,
        persisted.raw,
        persisted.recipients,
        30 * 1024 * 1024,
    )
    .map_err(|_| dakia_storage::Error::Invalid)
}
async fn deliver(
    store: &Store,
    a: &AuthorizedAccount,
    provider: &dyn MailProvider,
    s: &Submission,
) -> dakia_storage::Result<()> {
    let prepared = match prepare(store, a, s).await {
        Ok(p) => p,
        Err(_) => {
            store
                .transition(a, &s.id, SubmissionState::Claimed, SubmissionState::Failed)
                .await?;
            return Ok(());
        }
    };
    if let Some(graph) = provider.graph() {
        if store.provider_document(a,&format!("send:attempts:{}",s.id)).await?.is_some() && graph.has_sent_message(
            &store.mime(a,&s.id).await?.ok_or(dakia_storage::Error::NotFound)?.message_id
        ).await.unwrap_or(false) {
            store.transition(a,&s.id,SubmissionState::Claimed,SubmissionState::Submitting).await?;
            store.transition(a,&s.id,SubmissionState::Submitting,SubmissionState::Accepted).await?;
            return Ok(());
        }
    }
    let outcome = provider
        .submit(
            &prepared,
            &Gate {
                store,
                account: a,
                id: &s.id,
            },
        )
        .await;
    let current = store.submission(a, &s.id).await?;
    if current.state == SubmissionState::Cancelled {
        return Ok(());
    }
    let next = match outcome {
        p::SubmissionOutcome::Accepted { .. } => SubmissionState::Accepted,
        p::SubmissionOutcome::DeliveryUnknown => SubmissionState::DeliveryUnknown,
        p::SubmissionOutcome::Deferred { retry_after_secs } => {
            store.defer_submission(a,&s.id,current.state,retry_after_secs).await?;
            return Ok(());
        }
        p::SubmissionOutcome::Failed { kind } => {
            if kind == p::ErrorKind::Authentication {
                store.account_state(a, AccountState::Reconnect).await?;
            }
            SubmissionState::Failed
        }
    };
    store.transition(a, &s.id, current.state, next).await?;
    Ok(())
}
async fn file(
    store: &Store,
    a: &AuthorizedAccount,
    provider: &dyn MailProvider,
) -> dakia_storage::Result<()> {
    for s in store.needs_filing(a).await? {
        let Some(mime) = store.mime(a, &s.id).await? else {
            continue;
        };
        let status = match provider.reconcile_sent(&mime.message_id, &mime.raw).await {
            Ok(p::SentOutcome::ProviderManaged) => FilingStatus::NotRequired,
            Ok(p::SentOutcome::AlreadyPresent | p::SentOutcome::Appended) => FilingStatus::Filed,
            // A lost APPEND acknowledgment must be reconciled by Message-ID, never resubmitted.
            Ok(p::SentOutcome::FilingUnknown) | Err(_) => FilingStatus::Failed,
        };
        store.set_filing(a, &s.id, status).await?;
    }
    Ok(())
}
pub async fn run(app: App) {
    let mut recovered = std::collections::HashSet::new();
    let mut metadata = tokio::task::JoinSet::new();
    let mut active = std::collections::HashSet::new();
    let mut task_accounts = std::collections::HashMap::new();
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    let mut first_account = 0usize;
    let mut turns = std::collections::HashMap::<String, usize>::new();
    let mut eligible = std::collections::HashSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let maintenance = tokio::select! {
            // Due serial ownership takes priority over metadata completions.
            biased;
            _ = interval.tick() => true,
            Some(result) = metadata.join_next_with_id(), if !metadata.is_empty() => {
                let (task_id, progressed) = match result {
                    Ok((id, progressed)) => (id, progressed),
                    Err(error) => (error.id(), false),
                };
                if let Some(account) = task_accounts.remove(&task_id) {
                    active.remove(&account);
                    if !progressed {
                        turns.insert(account, METADATA_TURNS_PER_WINDOW);
                    }
                }
                false
            }
        };
        if maintenance {
            turns.clear();
            eligible.clear();
        }
        if !app.store.jobs_enabled().await.unwrap_or(false) {
            continue;
        }
        let Ok(mut accounts) = app.store.job_accounts().await else {
            continue;
        };
        if !accounts.is_empty() {
            let count = accounts.len();
            accounts.rotate_left(first_account % count);
        }
        let mut next_account = first_account;
        for (offset, a) in accounts.into_iter().enumerate() {
            if maintenance {
                if !recovered.contains(a.id()) {
                    if app.store.recover_mutations(&a).await.is_err() {
                        continue;
                    }
                    recovered.insert(a.id().to_owned());
                }
                // Completion-driven turns cannot bypass failed serial work.
                if tick(&app, &a).await.is_err() {
                    continue;
                }
                eligible.insert(a.id().to_owned());
            }
            // Only metadata runs concurrently. Submission recovery, claims,
            // delivery, filing and mutation leases retain their serial executor.
            let used = turns.get(a.id()).copied().unwrap_or(0);
            if eligible.contains(a.id())
                && !active.contains(a.id())
                && used < METADATA_TURNS_PER_WINDOW
            {
                if let Ok(permit) = slots.clone().try_acquire_owned() {
                    let id = a.id().to_owned();
                    turns.insert(id.clone(), used + 1);
                    // Admission resumes after the last admitted account,
                    // never preferentially at the fastest completing one.
                    next_account = first_account.wrapping_add(offset + 1);
                    active.insert(id.clone());
                    let app = app.clone();
                    let handle = metadata.spawn(async move {
                        let _permit = permit;
                        let progressed = crate::sync::tick(&app, &a).await.unwrap_or(false);
                        // Filtered discovery retains its original admission.
                        // Body acquisition has its own bounded page and shares
                        // this job's finite per-account/global admission.
                        if used == 0 {
                            let _ = crate::sync::starred::tick(&app, &a).await;
                        }
                        let bodies = crate::sync::full_text::tick(&app, &a)
                            .await
                            .unwrap_or(false);
                        progressed || bodies
                    });
                    task_accounts.insert(handle.id(), id);
                }
            }
        }
        first_account = next_account;
    }
}
// Six metadata/head/body jobs per account per five-second admission window. Two
// globally active, one per account. Idle/error waits until the next window;
// successful work immediately refills free slots within this finite budget.
const METADATA_TURNS_PER_WINDOW: usize = 6;

pub(crate) async fn tick(app: &App, a: &AuthorizedAccount) -> dakia_storage::Result<()> {
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let cleanup_at = app
        .store
        .provider_document(a, "attachments:cleanup-at")
        .await?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if Utc::now().timestamp() - cleanup_at >= 3600 {
        app.store.cleanup_orphan_attachments(a).await?;
        app.store
            .put_provider_document(
                a,
                "attachments:cleanup-at",
                &Utc::now().timestamp().to_string(),
            )
            .await?;
    }
    app.store.recover_expired(a, &now).await?;
    app.store.wake_snoozes(a).await?;
    let provider = app
        .mail
        .provider(&app.store, a)
        .await
        .map_err(|_| dakia_storage::Error::Invalid)?;
    if let Some(s) = app.store.due_submission(a, &now).await? {
        deliver(&app.store, a, provider.as_ref(), &s).await?;
    }
    if let Some(graph) = provider.graph() {
        for submission in app.store.uncertain_submissions(a).await? {
            let key = format!("send:lookup:{}",submission.id);
            let last = app.store.provider_document(a,&key).await?.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
            if last>Utc::now().timestamp()-60 { continue; }
            app.store.put_provider_document(a,&key,&Utc::now().timestamp().to_string()).await?;
            if let Some(mime) = app.store.mime(a,&submission.id).await? {
                if graph.has_sent_message(&mime.message_id).await.unwrap_or(false) {
                    app.store.confirm_delivery(a,&submission.id).await?;
                }
            }
        }
    }
    file(&app.store, a, provider.as_ref()).await?;
    crate::mutations::drain(app, a).await?;
    Ok(())
}
