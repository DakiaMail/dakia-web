//! Server-local inference. No account paths, network calls or result caches.
pub mod classification;
pub mod signals;
use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
pub use classification::{EmailClassificationInput, LocalEmailClassifier, ModelClassification};
use dakia_domain::{JobKind, JobStatus, ModelJob};

pub const POLICY_REVISION: &str = "2";

/// Trusted metadata loaded by the caller from its authorized account DB, never
/// constructed from a client request. Recheck this snapshot at durable commit.
pub struct ContentSnapshot<'a> {
    pub account_id: &'a str,
    pub message_id: &'a str,
    pub content_revision: u32,
    pub manual_category: Option<&'a str>,
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

/// Manual categories win without invoking the model. The caller must commit
/// conditionally on the same account/message/content/model/policy revisions,
/// and on manual ownership remaining unchanged. Failure leaves pending mail visible.
pub fn classify_job(
    classifier: &mut LocalEmailClassifier,
    job: &ModelJob,
    snapshot: &ContentSnapshot<'_>,
    input: EmailClassificationInput,
) -> Result<ModelClassification> {
    validate_job(job, snapshot, classifier.revision(), Utc::now())?;
    if let Some(category) = snapshot.manual_category {
        if ![
            "people",
            "transactions",
            "notifications",
            "newsletters",
            "other",
        ]
        .contains(&category)
        {
            bail!("invalid manual category");
        }
        return Ok(ModelClassification {
            category: category.into(),
            confidence: None,
        });
    }
    let result = classifier.classify(&[input])?.remove(0);
    validate_job(job, snapshot, classifier.revision(), Utc::now())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_correction_wins_and_invalid_correction_is_rejected() {
        let mut classifier = LocalEmailClassifier::from_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/email-classifier-v2"),
        )
        .unwrap();
        let mut j = job();
        j.model_revision = classifier.revision().into();
        let mut snapshot = ContentSnapshot {
            account_id: "a",
            message_id: "m",
            content_revision: 3,
            manual_category: Some("other"),
        };
        let input = || {
            EmailClassificationInput::new(
                None,
                "billing@example.test",
                "Payment receipt",
                "Your payment was received.",
                "",
            )
        };
        let result = classify_job(&mut classifier, &j, &snapshot, input()).unwrap();
        assert_eq!(result.category, "other");
        assert_eq!(result.confidence, None);
        snapshot.manual_category = Some("not-a-category");
        assert!(classify_job(&mut classifier, &j, &snapshot, input()).is_err());
        snapshot.manual_category = Some("people");
        j.account_id = "another-account".into();
        assert!(classify_job(&mut classifier, &j, &snapshot, input()).is_err());
    }

    fn job() -> ModelJob {
        ModelJob {
            id: "j".into(),
            account_id: "a".into(),
            message_id: "m".into(),
            content_revision: 3,
            model_revision: "r".into(),
            kind: JobKind::Classify,
            status: JobStatus::Running,
            deadline: "2099-01-01T00:00:00Z".into(),
        }
    }
    #[test]
    fn jobs_reject_other_accounts_stale_content_model_and_expiration() {
        let mut j = job();
        let s = ContentSnapshot {
            account_id: "a",
            message_id: "m",
            content_revision: 3,
            manual_category: None,
        };
        assert!(validate_job(&j, &s, "r", Utc::now()).is_ok());
        j.account_id = "b".into();
        assert!(validate_job(&j, &s, "r", Utc::now()).is_err());
        j = job();
        j.content_revision = 2;
        assert!(validate_job(&j, &s, "r", Utc::now()).is_err());
        j = job();
        assert!(validate_job(&j, &s, "new", Utc::now()).is_err());
        j.deadline = "2000-01-01T00:00:00Z".into();
        assert!(validate_job(&j, &s, "r", Utc::now()).is_err());
    }
}
