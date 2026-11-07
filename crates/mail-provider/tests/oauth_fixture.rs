mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, smtp::SmtpProvider, *};
use std::sync::Arc;

struct Token;
#[async_trait::async_trait]
impl AccessToken for Token {
    async fn access_token(&self) -> Result<String> {
        Ok("synthetic-access-token".into())
    }
}
// Base64 of user=fixture\x01auth=Bearer synthetic-access-token\x01\x01.
const BEARER: &str = "dXNlcj1maXh0dXJlAWF1dGg9QmVhcmVyIHN5bnRoZXRpYy1hY2Nlc3MtdG9rZW4BAQ==";

#[tokio::test]
async fn imap_xoauth2_uses_bearer_and_cancels_error_challenge_without_login_fallback() {
    for reject in [false, true] {
        let f = Fixture::new().await;
        let provider = ImapProvider::new(f.config(Security::Tls), credentials())
            .unwrap()
            .with_oauth(Arc::new(Token))
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap();
        let server = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            reply(&mut w, "* OK fixture\r\n").await;
            let request = line(&mut w).await;
            let (tag, request_command) = request.trim_end().split_once(' ').unwrap();
            assert_eq!(request_command, "AUTHENTICATE XOAUTH2");
            reply(&mut w, "+ \r\n").await;
            assert_eq!(line(&mut w).await.trim_end(), BEARER);
            if reject {
                reply(&mut w, "+ eyJzdGF0dXMiOiI0MDEifQ==\r\n").await;
                assert_eq!(line(&mut w).await, "\r\n");
                reply(&mut w, &format!("{tag} NO authentication failed\r\n")).await;
            } else {
                reply(&mut w, &format!("{tag} OK authenticated\r\n")).await;
                command_caps(&mut w).await;
                command(
                    &mut w,
                    "LIST \"\" *",
                    "* LIST () \"/\" \"INBOX\"\r\n$TAG OK list\r\n",
                )
                .await;
            }
        });
        let result = provider.probe().await;
        if reject {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Authentication);
        } else {
            assert!(result.is_ok(), "{result:?}");
        }
        server.await.unwrap();
    }
}
async fn command_caps(w: &mut Wire) {
    command(
        w,
        "CAPABILITY",
        "* CAPABILITY IMAP4rev1 AUTH=XOAUTH2\r\n$TAG OK caps\r\n",
    )
    .await;
}

#[tokio::test]
async fn smtp_xoauth2_probe_authenticates_without_sending_mail() {
    let f = Fixture::new().await;
    let provider = SmtpProvider::new(f.config(Security::Tls), credentials())
        .unwrap()
        .with_oauth(Arc::new(Token))
        .with_root(
            lettre::transport::smtp::client::Certificate::from_pem(f.pem.as_bytes()).unwrap(),
        )
        .unwrap();
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, true).await;
        reply(&mut w, "220 fixture ESMTP\r\n").await;
        assert!(line(&mut w).await.starts_with("EHLO "));
        reply(&mut w, "250-localhost\r\n250 AUTH XOAUTH2 PLAIN\r\n").await;
        assert_eq!(
            line(&mut w).await.trim_end(),
            format!("AUTH XOAUTH2 {BEARER}")
        );
        reply(&mut w, "235 authenticated\r\n").await;
        assert_eq!(line(&mut w).await, "QUIT\r\n");
        reply(&mut w, "221 bye\r\n").await;
    });
    provider.probe().await.unwrap();
    server.await.unwrap();
}

struct PendingToken;
#[async_trait::async_trait]
impl AccessToken for PendingToken {
    async fn access_token(&self) -> Result<String> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn token_renewal_obeys_smtp_deadline_without_auth_or_mail() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.timeout = std::time::Duration::from_millis(500);
    let provider = SmtpProvider::new(config, credentials())
        .unwrap()
        .with_oauth(Arc::new(PendingToken))
        .with_root(
            lettre::transport::smtp::client::Certificate::from_pem(f.pem.as_bytes()).unwrap(),
        )
        .unwrap();
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, true).await;
        reply(&mut w, "220 fixture ESMTP\r\n").await;
        assert!(line(&mut w).await.starts_with("EHLO "));
        reply(&mut w, "250-localhost\r\n250 AUTH XOAUTH2\r\n").await;
        assert!(
            line(&mut w).await.is_empty(),
            "no AUTH or MAIL after token timeout"
        );
    });
    let error = provider.probe().await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Retryable);
    assert_eq!(error.message, "SMTP deadline exceeded before message body");
    server.await.unwrap();
}
