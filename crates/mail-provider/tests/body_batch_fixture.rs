//! Real loopback TLS, bounded small-body acquisition and fail-closed checks.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};

fn reference(uid: u32) -> MessageRef {
    MessageRef {
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid,
    }
}
fn body(uid: u32, section: &str, raw: &str) -> String {
    format!(
        "* 1 FETCH (UID {uid} {section} {{{}}}\r\n{raw})\r\n",
        raw.len()
    )
}
#[tokio::test]
async fn body_batch_orders_by_requested_uid_and_never_returns_partial_or_wrong_content() {
    for scenario in [
        "ok",
        "tag",
        "truncated",
        "section",
        "missing",
        "duplicate",
        "large",
        "epoch",
    ] {
        let f = Fixture::new().await;
        let mut config = f.config(Security::Tls);
        config.kind = ProviderKind::Imap;
        let p = ImapProvider::new(config, credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap();
        let server = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            imap_login(&mut w, false).await;
            let epoch = if scenario == "epoch" { 99 } else { 17 };
            command(
                &mut w,
                "EXAMINE \"INBOX\"",
                &format!("* OK [UIDVALIDITY {epoch}] epoch\r\n$TAG OK done\r\n"),
            )
            .await;
            if scenario == "epoch" {
                return;
            }
            let size = if scenario == "large" { 65537 } else { 7 };
            command(&mut w, "UID FETCH 9,42 (UID RFC822.SIZE)", &format!("* 1 FETCH (UID 42 RFC822.SIZE 7)\r\n* 2 FETCH (UID 9 RFC822.SIZE {size})\r\n$TAG OK done\r\n")).await;
            if scenario == "large" {
                return;
            }
            let mut data = body(99, "BODY[]<0>", "ignored");
            data += &body(42, "BODY[]<0>", "body-42");
            if scenario != "missing" {
                let section = if scenario == "section" {
                    "BODY[HEADER]<0>"
                } else {
                    "BODY[]<0>"
                };
                let raw = if scenario == "truncated" {
                    "short"
                } else {
                    "body-09"
                };
                data += &body(9, section, raw);
            }
            if scenario == "duplicate" {
                data += &body(9, "BODY[]<0>", "changed");
            }
            data += if scenario == "tag" {
                "$TAG NO [LIMIT] failed\r\n"
            } else {
                "$TAG OK done\r\n"
            };
            command(&mut w, "UID FETCH 9,42 (UID BODY.PEEK[]<0.65537>)", &data).await;
        });
        let result = p
            .fetch_batch(&[reference(9), reference(42)], 4 * 1024 * 1024)
            .await;
        match scenario {
            "ok" => assert_eq!(
                result.unwrap(),
                vec![b"body-09".to_vec(), b"body-42".to_vec()]
            ),
            "large" => assert!(matches!(result, Err(e) if e.kind == ErrorKind::Unsupported)),
            "missing" | "epoch" => {
                assert!(matches!(result, Err(e) if e.kind == ErrorKind::StaleReference))
            }
            _ => assert!(matches!(result, Err(e) if e.kind == ErrorKind::Retryable)),
        }
        drop(p);
        server.await.unwrap();
    }
}
#[tokio::test]
async fn body_batch_rejects_cross_account_duplicates_and_large_scope_without_connecting() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = ProviderKind::Imap;
    let p = ImapProvider::new(config, credentials()).unwrap();
    let mut other = reference(42);
    other.account_id = "other".into();
    for refs in [
        vec![reference(9), other],
        vec![reference(9), reference(9)],
        (1..=51).map(reference).collect(),
    ] {
        assert!(
            matches!(p.fetch_batch(&refs, 4 * 1024 * 1024).await, Err(e) if e.kind == ErrorKind::InvalidInput)
        );
    }
}
