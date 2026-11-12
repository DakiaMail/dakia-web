//! Server-local inference. No account paths, network calls or result caches.
pub mod classification;
pub mod signals;
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
pub use classification::{EmailClassificationInput, LocalEmailClassifier, POLICY_REVISION};
use dakia_domain::{JobKind, JobStatus, ModelJob};

/// Trusted metadata loaded by the caller from its authorized account DB, never
/// constructed from a client request. Recheck this snapshot at durable commit.
pub struct ContentSnapshot<'a> {
    pub account_id: &'a str,
    pub message_id: &'a str,
    pub content_revision: u32,
}

pub fn validate_job(
    job: &ModelJob,
    snapshot: &ContentSnapshot<'_>,
    model_revision: &str,
    now: DateTime<Utc>,
) -> Result<()> {
    if job.account_id != snapshot.account_id
        || job.message_id != snapshot.message_id
        || job.content_revision != snapshot.content_revision
        || job.model_revision != model_revision
        || job.kind != JobKind::Classify
        || job.status != JobStatus::Running
    {
        bail!("stale or unauthorized classification job");
    }
    if DateTime::parse_from_rfc3339(&job.deadline)? <= now {
        bail!("classification deadline expired");
    }
    Ok(())
}

/// The caller conditionally commits against the same identity and revision,
/// then applies durable user corrections separately from these detected facts.
pub fn classify_job(
    classifier: &mut LocalEmailClassifier,
    job: &ModelJob,
    snapshot: &ContentSnapshot<'_>,
    input: EmailClassificationInput,
) -> Result<dakia_domain::Classification> {
    validate_job(job, snapshot, classifier.revision(), Utc::now())?;
    let result = classifier.classify(&[input])?.remove(0);
    validate_job(job, snapshot, classifier.revision(), Utc::now())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_job_still_rejects_foreign_stale_and_expired_results() {
        let snapshot = ContentSnapshot {
            account_id: "a",
            message_id: "m",
            content_revision: 3,
        };
        let job = ModelJob {
            id: "j".into(),
            account_id: "a".into(),
            message_id: "m".into(),
            content_revision: 3,
            model_revision: POLICY_REVISION.into(),
            kind: JobKind::Classify,
            status: JobStatus::Running,
            deadline: "2099-01-01T00:00:00Z".into(),
        };
        assert!(validate_job(&job, &snapshot, POLICY_REVISION, Utc::now()).is_ok());
        for bad in [
            ModelJob {
                account_id: "foreign".into(),
                ..job.clone()
            },
            ModelJob {
                content_revision: 2,
                ..job.clone()
            },
            ModelJob {
                model_revision: "old".into(),
                ..job.clone()
            },
            ModelJob {
                deadline: "2000-01-01T00:00:00Z".into(),
                ..job.clone()
            },
            ModelJob {
                status: JobStatus::Pending,
                ..job
            },
        ] {
            assert!(validate_job(&bad, &snapshot, POLICY_REVISION, Utc::now()).is_err());
        }
    }
}
