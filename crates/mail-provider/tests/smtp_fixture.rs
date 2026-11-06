mod common;
use common::*;
use dakia_mail_provider::{mime::assemble, smtp::SmtpProvider, *};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

async fn send_case(
    security: Security,
    reject_recipient: bool,
    lost_ack: bool,
    explicit_rejection: bool,
) {
    let fixture = Fixture::new().await;
    let config = fixture.config(security);
    let smtp = SmtpProvider::new(config.clone(), credentials())
        .unwrap()
        .with_root(
            lettre::transport::smtp::client::Certificate::from_pem(fixture.pem.as_bytes()).unwrap(),
        )
        .unwrap();
    let prepared = assemble(&config, compose(), 1024 * 1024).unwrap();
    let entered = Arc::new(AtomicBool::new(false));
    let server_entered = entered.clone();
    let server = tokio::spawn(async move {
        let mut w = fixture.accept(security == Security::StartTls, true).await;
        smtp_login(&mut w, security == Security::StartTls).await;
        assert!(line(&mut w)
            .await
            .starts_with("MAIL FROM:<sender@example.test>"));
        reply(&mut w, "250 sender ok\r\n").await;
        assert!(line(&mut w).await.starts_with("RCPT TO:<to@example.test>"));
        reply(&mut w, "250 recipient ok\r\n").await;
        assert!(line(&mut w)
            .await
            .starts_with("RCPT TO:<hidden@example.test>"));
        if reject_recipient {
            reply(&mut w, "550 rejected\r\n").await;
            assert_eq!(line(&mut w).await, "");
            assert!(!server_entered.load(Ordering::SeqCst));
            return None;
        }
        reply(&mut w, "250 recipient ok\r\n").await;
        assert_eq!(line(&mut w).await, "DATA\r\n");
        assert!(server_entered.load(Ordering::SeqCst));
        reply(&mut w, "354 send body\r\n").await;
        let mut raw = String::new();
        loop {
            let l = line(&mut w).await;
            assert!(!l.is_empty());
            if l == ".\r\n" {
                break;
            }
            raw.push_str(&l);
        }
        if !lost_ack {
            reply(
                &mut w,
                if explicit_rejection {
                    "554 policy rejected\r\n"
                } else {
                    "250 queued fixture-id\r\n"
                },
            )
            .await;
        }
        Some(raw)
    });
    let outcome = smtp
        .submit(&prepared, || async {
            entered.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
    let captured = server.await.unwrap();
    if reject_recipient || explicit_rejection {
        assert!(matches!(outcome, SubmissionOutcome::Failed { .. }));
    } else if lost_ack {
        assert_eq!(outcome, SubmissionOutcome::DeliveryUnknown);
    } else {
        assert!(matches!(outcome, SubmissionOutcome::Accepted { .. }));
    }
    if let Some(raw) = captured {
        let parsed = mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .unwrap();
        assert!(parsed.bcc().is_none());
        assert!(raw.contains("In-Reply-To: <parent@example.test>"));
        assert!(raw.contains("References: <ancestor@example.test> <parent@example.test>"));
        assert_eq!(parsed.body_text(0).unwrap().trim(), "plain fixture");
        assert!(parsed.body_html(0).unwrap().contains("HTML fixture"));
        assert_eq!(parsed.attachments().count(), 1);
        assert_eq!(
            parsed.attachments().next().unwrap().contents(),
            b"attachment bytes\nLF\r\nCRLF"
        );
    }
}
#[tokio::test]
async fn tls_delivered_mime_bcc_attachment_and_gate() {
    send_case(Security::Tls, false, false, false).await;
}
#[tokio::test]
async fn starttls_delivered_mime() {
    send_case(Security::StartTls, false, false, false).await;
}
#[tokio::test]
async fn rejected_recipient_aborts_all_before_data() {
    send_case(Security::Tls, true, false, false).await;
}
#[tokio::test]
async fn lost_final_ack_is_unknown_not_retryable() {
    send_case(Security::Tls, false, true, false).await;
}
#[tokio::test]
async fn explicit_post_data_rejection_is_known_failure() {
    send_case(Security::Tls, false, false, true).await;
}
#[tokio::test]
async fn invalid_tls_never_authenticates() {
    let fixture = Fixture::new().await;
    let smtp = SmtpProvider::new(fixture.config(Security::Tls), credentials()).unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = fixture.listener.accept().await.unwrap();
        assert!(fixture.acceptor.accept(tcp).await.is_err());
    });
    assert!(smtp.probe().await.is_err());
    server.await.unwrap();
}
#[test]
fn attachment_line_endings_are_byte_exact() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fixture = rt.block_on(Fixture::new());
    let mut draft = compose();
    let bytes = b"Synthetic attachment\nLF\r\nCRLF\rCR\0binary".to_vec();
    draft.attachments[0].bytes = bytes.clone();
    draft.attachments[0].content_type = "text/plain".into();
    let prepared = assemble(&fixture.config(Security::Tls), draft, 1024 * 1024).unwrap();
    let parsed = mail_parser::MessageParser::default()
        .parse(prepared.raw())
        .unwrap();
    assert_eq!(parsed.attachments().next().unwrap().contents(), bytes);
    assert!(String::from_utf8_lossy(prepared.raw()).contains("Content-Transfer-Encoding: base64"));
}
#[test]
fn mime_limits_and_header_injection() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fixture = rt.block_on(Fixture::new());
    let config = fixture.config(Security::Tls);
    let mut draft = compose();
    draft.subject = "bad\r\nBcc: attacker@example.test".into();
    assert!(assemble(&config, draft, 1024 * 1024).is_err());
    assert!(assemble(&config, compose(), 10).is_err());
    let mut draft = compose();
    draft.message_id = "regenerate-on-retry".into();
    assert!(assemble(&config, draft, 1024 * 1024).is_err());
    let prepared = assemble(&config, compose(), 1024 * 1024).unwrap();
    let restored = dakia_mail_provider::mime::PreparedMessage::restore(
        &config,
        prepared.raw().to_vec(),
        prepared.recipients(),
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(restored.raw(), prepared.raw());
    assert_eq!(restored.recipients(), prepared.recipients());
    let mut other = config.clone();
    other.primary_address = "other@example.test".into();
    assert!(dakia_mail_provider::mime::PreparedMessage::restore(
        &other,
        prepared.raw().to_vec(),
        prepared.recipients(),
        1024 * 1024
    )
    .is_err());
}
#[test]
fn parsed_bare_reply_ids_are_written_as_rfc_message_ids() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fixture = rt.block_on(Fixture::new());
    let config = fixture.config(Security::Tls);
    let mut draft = compose();
    draft.in_reply_to = Some("parent@example.test".into());
    draft.references = Some("ancestor@example.test <parent@example.test>".into());
    let prepared = assemble(&config, draft, 1024 * 1024).unwrap();
    let raw = String::from_utf8_lossy(prepared.raw());
    assert!(raw.contains("In-Reply-To: <parent@example.test>\r\n"));
    assert!(raw.contains("References: <ancestor@example.test> <parent@example.test>\r\n"));
    for invalid in [
        "no-at-sign",
        "<parent@example.test",
        "a@example.test b@example.test",
    ] {
        let mut draft = compose();
        draft.in_reply_to = Some(invalid.into());
        assert!(assemble(&config, draft, 1024 * 1024).is_err());
    }
}
#[tokio::test]
async fn probe_does_not_submit_an_envelope() {
    let fixture = Fixture::new().await;
    let smtp = SmtpProvider::new(fixture.config(Security::StartTls), credentials())
        .unwrap()
        .with_root(
            lettre::transport::smtp::client::Certificate::from_pem(fixture.pem.as_bytes()).unwrap(),
        )
        .unwrap();
    let server = tokio::spawn(async move {
        let mut w = fixture.accept(true, true).await;
        smtp_login(&mut w, true).await;
        assert_eq!(line(&mut w).await, "QUIT\r\n");
        reply(&mut w, "221 bye\r\n").await;
    });
    smtp.probe().await.unwrap();
    server.await.unwrap();
}
