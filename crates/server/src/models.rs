use crate::{ApiFailure, App};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use dakia_domain::*;
use dakia_inference::{ContentSnapshot, EmailClassificationInput, LocalEmailClassifier};
use dakia_storage::{AuthorizedAccount, Error, ModelFailureReason};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::Instrument;
fn workers() -> PathBuf {
    std::env::var_os("DAKIA_WORKERS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("workers"))
}
fn translation_revision() -> Result<String, Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(std::fs::read(workers().join("translation/models.json"))?)
    ))
}
#[derive(Deserialize)]
pub(crate) struct Language {
    language: String,
}
fn source(language: &str) -> crate::Result<()> {
    static SOURCES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let sources = SOURCES.get_or_init(|| {
        let registry: serde_json::Value =
            serde_json::from_str(include_str!("../../../workers/translation/models.json"))
                .expect("bundled translation registry");
        registry["models"]
            .as_array()
            .expect("registered models")
            .iter()
            .filter(|model| model["target"] == "en")
            .filter_map(|model| model["source"].as_str().map(str::to_owned))
            .collect()
    });
    if sources.iter().any(|source| source == language) {
        Ok(())
    } else {
        Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "unsupported_source_language",
        ))
    }
}
pub(crate) async fn queue(
    State(app): State<App>,
    Extension(user): Extension<crate::Identity>,
    Path((account, id)): Path<(String, String)>,
    Query(query): Query<Language>,
) -> crate::Result<Json<ModelJob>> {
    let a = app.store.authorize(&user.user, &account).await?;
    source(&query.language)?;
    let revision = translation_revision()?;
    Ok(Json(
        app.store
            .queue_model(
                &a,
                &id,
                JobKind::Translate,
                &revision,
                Some(&query.language),
            )
            .await?,
    ))
}
pub(crate) async fn translated(
    State(app): State<App>,
    Extension(user): Extension<crate::Identity>,
    Path((account, id)): Path<(String, String)>,
    Query(query): Query<Language>,
) -> crate::Result<Json<MessageContent>> {
    let a = app.store.authorize(&user.user, &account).await?;
    source(&query.language)?;
    app.store.message(&a, &id).await?;
    let raw = app
        .store
        .provider_document(&a, &format!("translation:{id}:{}", query.language))
        .await?
        .ok_or(ApiFailure(StatusCode::CONFLICT, "translation_not_ready"))?;
    let mut content: MessageContent = serde_json::from_str(&raw).map_err(Error::from)?;
    content.message = app.store.message(&a, &id).await?;
    Ok(Json(content))
}
pub(crate) async fn job(
    State(app): State<App>,
    Extension(user): Extension<crate::Identity>,
    Path((account, id)): Path<(String, String)>,
) -> crate::Result<Json<ModelJob>> {
    let a = app.store.authorize(&user.user, &account).await?;
    Ok(Json(app.store.model_job(&a, &id).await?))
}
/// Fresh bounded subprocess per request. Death cannot leak account state to the next job.
pub(crate) async fn node_worker(
    kind: &str,
    input: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let mut child =
        tokio::process::Command::new(std::env::var("DAKIA_NODE").unwrap_or_else(|_| "node".into()))
            .arg(workers().join(kind).join("main.mjs"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
    let result = tokio::time::timeout(Duration::from_secs(125), async {
        let mut stdin = child.stdin.take().ok_or(Error::Invalid)?;
        let mut bytes = serde_json::to_vec(&input)?;
        bytes.push(b'\n');
        stdin.write_all(&bytes).await?;
        drop(stdin);
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or(Error::Invalid)?
            .take(2 * 1024 * 1024)
            .read_to_end(&mut output)
            .await?;
        if !child.wait().await?.success() {
            return Err(Error::Invalid);
        }
        for line in output.split(|b| *b == b'\n').rev() {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) {
                if let Some(result) = value.get("result") {
                    return Ok(result.clone());
                }
                if value.get("error").is_some() {
                    return Err(Error::Invalid);
                }
            }
        }
        Err(Error::Invalid)
    })
    .await
    .map_err(|_| Error::Invalid)?;
    result
}
pub async fn run(app: App) {
    let path = std::env::var("DAKIA_CLASSIFIER_DIR")
        .unwrap_or_else(|_| "crates/inference/assets/email-classifier-v2".into());
    let classifier =
        tokio::task::spawn_blocking(move || LocalEmailClassifier::from_dir(path)).await;
    let (classifier, revision) = match classifier {
        Ok(Ok(classifier)) => {
            let revision = classifier.revision().to_owned();
            (Some(Arc::new(Mutex::new(classifier))), revision)
        }
        _ => {
            tracing::warn!("classification runtime unavailable");
            (None, String::new())
        }
    };
    let mut recovered = std::collections::HashSet::new();
    loop {
        if app.store.jobs_enabled().await.unwrap_or(false) {
            if let Ok(accounts) = app.store.job_accounts().await {
                for a in accounts {
                    if !recovered.contains(a.id()) {
                        if app.store.recover_models(&a).await.is_err() {
                            continue;
                        }
                        recovered.insert(a.id().to_owned());
                    }
                    if classifier.is_some() {
                        if let Ok(Some(message)) = app.store.classification_candidate(&a).await {
                            let _ = app
                                .store
                                .queue_model(&a, &message.id, JobKind::Classify, &revision, None)
                                .await;
                        }
                    }
                    if let Ok(Some(mut job)) = app.store.claim_model(&a).await {
                        let mut reason = ModelFailureReason::WorkerFailed;
                        let result = execute(&app, &a, &job, classifier.clone(), &mut reason)
                            .instrument(tracing::info_span!(
                                target: "dakia_server::diagnostics",
                                "model_attempt",
                                purpose = if job.kind == JobKind::Classify { "classification" } else { "translation" },
                                message_key = crate::diagnostics::correlation(&job.message_id),
                                account_key = crate::diagnostics::correlation(a.id()),
                                job_key = crate::diagnostics::correlation(&job.id)
                            ))
                            .await;
                        if result.is_err() {
                            let deferred = app.store.defer_model_failure(&a, &job, reason).await;
                            tracing::warn!(target: "dakia_server::diagnostics",
                                account_key = crate::diagnostics::correlation(a.id()),
                                job_key = crate::diagnostics::correlation(&job.id),
                                reason = reason.code(), deferred = matches!(deferred, Ok(true)),
                                "model job attempt failed");
                            if matches!(deferred, Ok(true)) {
                                continue;
                            }
                        }
                        job.status = match &result {
                            Ok(Some(_)) => JobStatus::Succeeded,
                            Ok(None) => JobStatus::Cancelled,
                            Err(_) => JobStatus::Failed,
                        };
                        let (category, translation) = result.ok().flatten().unwrap_or((None, None));
                        let job_key = crate::diagnostics::correlation(&job.id);
                        if let Err(error) = app
                            .store
                            .finish_model(
                                &a,
                                job,
                                category.as_deref(),
                                translation.as_ref().map(|(s, c)| (s.as_str(), c)),
                            )
                            .await
                        {
                            tracing::warn!(target: "dakia_server::diagnostics",
                                account_key = crate::diagnostics::correlation(a.id()), job_key,
                                error = %error, "model job outcome not persisted");
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
async fn execute(
    app: &App,
    a: &AuthorizedAccount,
    job: &ModelJob,
    classifier: Option<Arc<Mutex<LocalEmailClassifier>>>,
    reason: &mut ModelFailureReason,
) -> Result<Option<(Option<String>, Option<(String, MessageContent)>)>, Error> {
    if job.kind == JobKind::Classify && !app.store.message(a, &job.message_id).await?.unread {
        return Ok(None);
    }
    if chrono::DateTime::parse_from_rfc3339(&job.deadline).map_err(|_| Error::Invalid)?
        <= chrono::Utc::now()
    {
        *reason = ModelFailureReason::Deadline;
        return Err(Error::Invalid);
    }
    let content = app
        .mail
        .content(&app.store, a, &job.message_id)
        .await
        .map_err(|error| {
            *reason = content_failure_reason(&error);
            Error::Invalid
        })?;
    if job.kind == JobKind::Classify && !app.store.message(a, &job.message_id).await?.unread {
        return Ok(None);
    }
    if content.message.revision != job.content_revision {
        *reason = ModelFailureReason::RevisionChanged;
        return Err(Error::Conflict);
    }
    if job.kind == JobKind::Translate {
        let source = app
            .store
            .provider_document(a, &format!("job-source:{}", job.id))
            .await?
            .ok_or(Error::Invalid)?;
        let html = !content.html.is_empty();
        let result=node_worker("translation",serde_json::json!({"job":job,"scope":{"account_id":a.id(),"message_id":job.message_id,"content_revision":job.content_revision},"source":source,"text":if html {&content.html}else{&content.text},"html":html})).await?;
        if result["account_id"] != job.account_id
            || result["message_id"] != job.message_id
            || result["job_id"] != job.id
            || result["content_revision"] != job.content_revision
            || result["model_revision"] != job.model_revision
        {
            return Err(Error::Invalid);
        }
        let translated = result["text"].as_str().ok_or(Error::Invalid)?.to_owned();
        let mut content = content;
        if html {
            content.html = translated;
            content.text.clear();
        } else {
            content.text = translated;
            content.html.clear();
        }
        return Ok(Some((None, Some((source, content)))));
    }
    let raw = app
        .mail
        .raw(&app.store, a, &job.message_id)
        .await
        .map_err(|error| {
            *reason = content_failure_reason(&error);
            Error::Invalid
        })?;
    if !app.store.message(a, &job.message_id).await?.unread {
        return Ok(None);
    }
    let parsed = mail_parser::MessageParser::default()
        .parse(&raw)
        .ok_or(Error::Invalid)?;
    let signals =
        dakia_inference::signals::classification_signals(&parsed, &content.message.folder_ids);
    let input = EmailClassificationInput::new(
        content.message.sender.name.as_deref(),
        &content.message.sender.email,
        &content.message.subject,
        &content.text,
        &signals,
    )
    .with_known_correspondence(
        app.store
            .known_recipient(a, &content.message.sender.email)
            .await?,
    );
    let job = job.clone();
    let classifier = classifier.ok_or(Error::Invalid)?;
    let category = tokio::task::spawn_blocking(move || {
        let snapshot = ContentSnapshot {
            account_id: &job.account_id,
            message_id: &job.message_id,
            content_revision: job.content_revision,
            manual_category: None,
        };
        let mut classifier = classifier.lock().map_err(|_| Error::Invalid)?;
        dakia_inference::classify_job(&mut classifier, &job, &snapshot, input)
            .map(|r| r.category)
            .map_err(|_| Error::Invalid)
    })
    .await
    .map_err(|_| Error::Invalid)??;
    Ok(Some((Some(category), None)))
}

fn content_failure_reason(error: &ApiFailure) -> ModelFailureReason {
    if error.0.is_server_error() && error.1 == "provider_unavailable" {
        ModelFailureReason::ContentUnavailable
    } else {
        ModelFailureReason::ContentInvalid
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[tokio::test]
    async fn read_after_claim_skips_content_fetch_and_classifier() {
        let dir = tempfile::tempdir().unwrap();
        let store = dakia_storage::Store::open(dir.path(), [4; 32])
            .await
            .unwrap();
        let user = store.user("issuer", "owner").await.unwrap();
        let input = serde_json::from_value(serde_json::json!({
            "email":"fixture@example.invalid","display_name":"Fixture","provider":"imap",
            "imap_host":"imap.invalid","imap_port":993,"imap_tls":"implicit",
            "smtp_host":"smtp.invalid","smtp_port":465,"smtp_tls":"implicit",
            "username":"fixture","password":"fixture-only"
        }))
        .unwrap();
        let account = store.connect(&user, &input).await.unwrap();
        let a = store.authorize(&user, &account.id).await.unwrap();
        let message: Message = serde_json::from_value(serde_json::json!({
            "id":"fixture","account_id":a.id(),"thread_id":"thread","provider_id":"stable",
            "folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},
            "to":[],"subject":"Fixture","preview":"","received_at":"2026-10-30T00:00:00Z",
            "unread":true,"starred":false,"category":null,"category_status":"pending","revision":0
        }))
        .unwrap();
        let mut message = store.upsert_message(&a, message).await.unwrap();
        store
            .queue_model(&a, &message.id, JobKind::Classify, "fixture", None)
            .await
            .unwrap();
        let mut job = store.claim_model(&a).await.unwrap().unwrap();
        // Simulate an external read flag arriving through sync after claim.
        message.unread = false;
        store.upsert_message(&a, message).await.unwrap();
        let app = App {
            store,
            mail: crate::mail::MailRuntime::default(),
            config: Arc::new(crate::Config {
                origin: "https://dakia.invalid".into(),
                issuer: "https://id.invalid".into(),
                client_id: "fixture".into(),
                client_secret: "fixture".into(),
                web_root: "missing".into(),
            }),
        };
        // No content, provider reference or classifier exists. Any content fetch
        // or inference would fail; a read message must instead skip successfully.
        let mut reason = ModelFailureReason::WorkerFailed;
        assert!(execute(&app, &a, &job, None, &mut reason)
            .await
            .unwrap()
            .is_none());
        // If it becomes unread again before finish, retain the skipped outcome
        // and let scheduling create a job with its new revision.
        let mut message = app.store.message(&a, &job.message_id).await.unwrap();
        message.unread = true;
        app.store.upsert_message(&a, message).await.unwrap();
        job.status = JobStatus::Cancelled;
        app.store
            .finish_model(&a, job.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(
            app.store.model_job(&a, &job.id).await.unwrap().status,
            JobStatus::Cancelled
        );
        assert!(app
            .store
            .classification_candidate(&a)
            .await
            .unwrap()
            .is_some());
    }

    #[test]
    fn every_registered_english_model_is_accepted_as_source() {
        let registry: serde_json::Value =
            serde_json::from_str(include_str!("../../../workers/translation/models.json")).unwrap();
        for model in registry["models"].as_array().unwrap() {
            assert!(
                source(model["source"].as_str().unwrap()).is_ok(),
                "{}",
                model["source"]
            );
        }
        for code in ["en", "unknown", "rus", "en-US", "", "xx"] {
            assert!(source(code).is_err());
        }
    }

    #[test]
    fn only_transient_provider_failures_allow_classification_retries() {
        for (status, code, expected) in [
            (
                StatusCode::BAD_GATEWAY,
                "provider_unavailable",
                "content_unavailable",
            ),
            (
                StatusCode::BAD_REQUEST,
                "provider_unavailable",
                "content_invalid",
            ),
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                "message_too_large",
                "content_invalid",
            ),
            (
                StatusCode::UNAUTHORIZED,
                "account_reconnect_required",
                "content_invalid",
            ),
            (
                StatusCode::BAD_GATEWAY,
                "invalid_attachment_encoding",
                "content_invalid",
            ),
        ] {
            assert_eq!(
                content_failure_reason(&ApiFailure(status, code)).code(),
                expected
            );
        }
    }
}
