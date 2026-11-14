mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::instrument::WithSubscriber;
fn reference() -> MessageRef {
    MessageRef {
        graph_id: None,
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid: 42,
    }
}
async fn select(w: &mut Wire) {
    command(
        w,
        "SELECT \"INBOX\"",
        "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
    )
    .await;
    command(w, "UID SEARCH UID 42", "* SEARCH 42\r\n$TAG OK search\r\n").await;
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);
impl Write for Logs {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Real TLS sockets, production codec and provider, not an intercepted API.
// These prove instrumentation and timeout semantics, not the historical cause.
async fn stalled_archive(stage: &'static str) {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    config.timeout = Duration::from_millis(800);
    let p = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap();
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        if stage == "select_reference" {
            assert!(line(&mut w).await.contains("SELECT"));
        } else {
            select(&mut w).await;
            command(
                &mut w,
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 MOVE X-GM-EXT-1\r\n$TAG OK caps\r\n",
            )
            .await;
            gmail_archive_reference(&mut w).await;
            if stage == "write" {
                assert!(line(&mut w).await.contains("UID STORE 55 -X-GM-LABELS"));
            } else {
                command(
                    &mut w,
                    "UID STORE 55 -X-GM-LABELS (\"\\\\Inbox\")",
                    "$TAG OK [COPYUID 18 42 101] moved\r\n",
                )
                .await;
                assert!(line(&mut w)
                    .await
                    .contains("UID FETCH 55 (UID X-GM-MSGID X-GM-LABELS)"));
            }
        }
        tokio::time::sleep(Duration::from_millis(900)).await;
    });
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let e = p
        .mutate(&reference(), Mutation::Archive)
        .with_subscriber(subscriber)
        .await
        .unwrap_err();
    assert_eq!(
        e.kind,
        if stage == "select_reference" {
            ErrorKind::Retryable
        } else {
            ErrorKind::Uncertain
        }
    );
    assert_eq!(e.message, "IMAP operation deadline exceeded");
    server.await.unwrap();
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for expected in [
        format!("stage=\"{stage}\""),
        "deadline_ms=800".into(),
        "elapsed_ms=".into(),
        "checkout_ms=".into(),
        format!(
            "outcome=\"{}\"",
            if stage == "select_reference" {
                "failed"
            } else {
                "unknown"
            }
        ),
        format!("write_started={}", stage != "select_reference"),
        format!("write_acknowledged={}", stage == "verification"),
    ] {
        assert!(text.contains(&expected), "missing {expected}: {text}");
    }
    for private in [
        "fixture-account",
        "fixture-only-password",
        "sender@example.test",
        "Archive",
        "UID MOVE",
    ] {
        assert!(!text.contains(private), "leaked {private}");
    }
}
#[tokio::test]
async fn deadline_before_write_preserves_reason_and_reports_no_write() {
    stalled_archive("select_reference").await;
}
#[tokio::test]
async fn lost_write_ack_is_unknown_not_success() {
    stalled_archive("write").await;
}
#[tokio::test]
async fn verification_timeout_keeps_ack_distinct_from_success() {
    stalled_archive("verification").await;
}
