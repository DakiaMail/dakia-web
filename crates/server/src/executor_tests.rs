use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
struct Provider {
    outcome: p::SubmissionOutcome,
    sends: AtomicUsize,
    filings: AtomicUsize,
    raw: Mutex<Vec<u8>>,
}
fn unsupported<T>() -> p::Result<T> {
    Err(p::ProviderError {
        kind: p::ErrorKind::Unsupported,
        message: "fixture".into(),
    })
}
#[async_trait]
impl MailProvider for Provider {
    fn account_id(&self) -> &str {
        "fixture"
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        unsupported()
    }
    async fn scan(
        &self,
        _: &str,
        _: &p::Search,
        _: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        unsupported()
    }
    async fn search(
        &self,
        _: &str,
        _: &p::Search,
        _: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        unsupported()
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        unsupported()
    }
    async fn fetch(&self, _: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        unsupported()
    }
    async fn mutate(&self, _: &p::MessageRef, _: p::Mutation) -> p::Result<p::MutationResult> {
        unsupported()
    }
    async fn submit(
        &self,
        prepared: &PreparedMessage,
        gate: &dyn SubmissionGate,
    ) -> p::SubmissionOutcome {
        if gate.begin_data().await.is_err() {
            return p::SubmissionOutcome::Failed {
                kind: p::ErrorKind::Retryable,
            };
        }
        self.sends.fetch_add(1, Ordering::SeqCst);
        *self.raw.lock().unwrap() = prepared.raw().to_vec();
        self.outcome.clone()
    }
    async fn reconcile_sent(&self, _: &str, raw: &[u8]) -> p::Result<p::SentOutcome> {
        assert_eq!(raw, self.raw.lock().unwrap().as_slice());
        Ok(if self.filings.fetch_add(1, Ordering::SeqCst) == 0 {
            p::SentOutcome::FilingUnknown
        } else {
            p::SentOutcome::AlreadyPresent
        })
    }
}
async fn setup() -> (tempfile::TempDir, Store, AuthorizedAccount, Submission) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [1; 32]).await.unwrap();
    let user = store
        .user("https://fixture.invalid", "owner")
        .await
        .unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let draft: Draft = serde_json::from_value(serde_json::json!({"id":"draft","account_id":a.id(),"revision":0,"to":[{"email":"recipient@example.invalid","name":null}],"cc":[],"bcc":[{"email":"private@example.invalid","name":null}],"subject":"Synthetic only","text":"Original snapshot","html":"<p>Original snapshot</p>","in_reply_to":null,"references":[],"attachments":[],"saved_at":""})).unwrap();
    let draft = store.save_draft(&a, draft).await.unwrap();
    let submission = store
        .submit(
            &a,
            &SubmitRequest {
                operation_id: "once".into(),
                draft_id: draft.id,
                expected_revision: draft.revision,
                due_at: None,
                timezone: "UTC".into(),
            },
        )
        .await
        .unwrap();
    let same = store
        .submit(
            &a,
            &SubmitRequest {
                operation_id: "once".into(),
                draft_id: submission.draft_id.clone(),
                expected_revision: submission.draft_revision,
                due_at: None,
                timezone: "UTC".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(same.id, submission.id);
    let claimed = store
        .due_submission(&a, "9999-01-01T00:00:00.000Z")
        .await
        .unwrap()
        .unwrap();
    (dir, store, a, claimed)
}
#[tokio::test]
async fn smtp_outcomes_are_durable_and_filing_never_resends() {
    for (outcome, expected) in [
        (
            p::SubmissionOutcome::Accepted {
                response: "ok".into(),
            },
            SubmissionState::Accepted,
        ),
        (
            p::SubmissionOutcome::DeliveryUnknown,
            SubmissionState::DeliveryUnknown,
        ),
        (
            p::SubmissionOutcome::Failed {
                kind: p::ErrorKind::Retryable,
            },
            SubmissionState::Failed,
        ),
    ] {
        let (_dir, store, a, s) = setup().await;
        let provider = Provider {
            outcome,
            sends: AtomicUsize::new(0),
            filings: AtomicUsize::new(0),
            raw: Mutex::new(Vec::new()),
        };
        let mut draft = store.draft(&a, "draft").await.unwrap();
        draft.text = "Newer draft must not be sent".into();
        store.save_draft(&a, draft).await.unwrap();
        deliver(&store, &a, &provider, &s).await.unwrap();
        assert_eq!(store.submission(&a, &s.id).await.unwrap().state, expected);
        assert!(store
            .due_submission(&a, "9999-01-01T00:00:00.000Z")
            .await
            .unwrap()
            .is_none());
        let bytes = provider.raw.lock().unwrap().clone();
        let mime = mail_parser::MessageParser::default().parse(&bytes).unwrap();
        assert!(mime.bcc().is_none());
        assert!(mime.body_text(0).unwrap().contains("Original snapshot"));
        assert_eq!(
            store
                .mime(&a, &s.id)
                .await
                .unwrap()
                .unwrap()
                .recipients
                .len(),
            2
        );
        file(&store, &a, &provider).await.unwrap();
        file(&store, &a, &provider).await.unwrap();
        assert_eq!(provider.sends.load(Ordering::SeqCst), 1);
        if expected == SubmissionState::Accepted {
            assert_eq!(
                store.submission(&a, &s.id).await.unwrap().filing_status,
                FilingStatus::Filed
            );
            assert_eq!(provider.filings.load(Ordering::SeqCst), 2);
        } else {
            assert_eq!(provider.filings.load(Ordering::SeqCst), 0);
        }
    }
}
#[tokio::test]
async fn cancel_before_data_prevents_send() {
    let (_dir, store, a, s) = setup().await;
    store
        .transition(
            &a,
            &s.id,
            SubmissionState::Claimed,
            SubmissionState::Cancelled,
        )
        .await
        .unwrap();
    let provider = Provider {
        outcome: p::SubmissionOutcome::Accepted {
            response: "ok".into(),
        },
        sends: AtomicUsize::new(0),
        filings: AtomicUsize::new(0),
        raw: Mutex::new(Vec::new()),
    };
    deliver(&store, &a, &provider, &s).await.unwrap();
    assert_eq!(provider.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.submission(&a, &s.id).await.unwrap().state,
        SubmissionState::Cancelled
    );
}
