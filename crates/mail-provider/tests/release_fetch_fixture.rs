//! Real TLS and IMAP responses, no real mailbox or external credentials.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};

async fn fetch(
    size: u32,
    bodies: Vec<(u32, &'static str)>,
    maximum: u32,
    tagged_ok: bool,
) -> Result<Vec<u8>> {
    fetch_section(size, bodies, maximum, tagged_ok, "BODY[]<0>").await
}

async fn fetch_section(
    size: u32,
    bodies: Vec<(u32, &'static str)>,
    maximum: u32,
    tagged_ok: bool,
    section: &'static str,
) -> Result<Vec<u8>> {
    let fixture = Fixture::new().await;
    let provider = ImapProvider::new(fixture.config(Security::Tls), credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&fixture.cert).unwrap())
        .unwrap();
    let task = tokio::spawn(async move {
        let mut wire = fixture.accept(false, false).await;
        imap_login(&mut wire, false).await;
        command(
            &mut wire,
            "SELECT \"INBOX\"",
            "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut wire,
            "UID SEARCH UID 9",
            "* SEARCH 9\r\n$TAG OK found\r\n",
        )
        .await;
        command(
            &mut wire,
            "UID FETCH 9 (UID RFC822.SIZE)",
            &format!("* 1 FETCH (UID 9 RFC822.SIZE {size})\r\n$TAG OK size\r\n"),
        )
        .await;
        if size > maximum {
            return;
        }
        let mut response = String::new();
        for (uid, body) in bodies {
            response += &format!(
                "* 1 FETCH (UID {uid} {section} {{{}}}\r\n{body})\r\n",
                body.len()
            );
        }
        response += if tagged_ok {
            "$TAG OK complete\r\n"
        } else {
            "$TAG NO unavailable\r\n"
        };
        command(
            &mut wire,
            &format!(
                "UID FETCH 9 (UID BODY.PEEK[]<0.{}>)",
                u64::from(maximum) + 1
            ),
            &response,
        )
        .await;
    });
    let result = provider
        .fetch(
            &MessageRef {
                account_id: "fixture-account".into(),
                folder: "INBOX".into(),
                uidvalidity: 17,
                uid: 9,
            },
            maximum,
        )
        .await;
    task.await.unwrap();
    result
}

#[tokio::test]
async fn exact_body_length_is_preserved_without_marking_read() {
    assert_eq!(
        fetch(7, vec![(9, "fixture")], 7, true).await.unwrap(),
        b"fixture"
    );
}
#[tokio::test]
async fn size_over_limit_rejects_before_requesting_body() {
    assert_eq!(
        fetch(8, vec![], 7, true).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}
#[tokio::test]
async fn oversized_actual_body_is_rejected_even_when_advertised_size_is_small() {
    assert_eq!(
        fetch(2, vec![(9, "too large")], 7, true)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
}
#[tokio::test]
async fn tagged_failure_after_literal_never_returns_partial_success() {
    assert!(fetch(7, vec![(9, "fixture")], 10, false).await.is_err());
}
#[tokio::test]
async fn missing_target_body_never_substitutes_unsolicited_uid() {
    assert!(fetch(7, vec![(10, "foreign")], 10, true).await.is_err());
}
#[tokio::test]
async fn unsolicited_body_before_target_does_not_replace_requested_message() {
    assert_eq!(
        fetch(7, vec![(10, "foreign"), (9, "fixture")], 10, true)
            .await
            .unwrap(),
        b"fixture"
    );
}
#[tokio::test]
async fn successful_tag_cannot_make_truncated_body_complete() {
    assert!(
        fetch(100, vec![(9, "partial")], 100, true).await.is_err(),
        "short literal was accepted as a complete message and could be cached permanently"
    );
}
#[tokio::test]
async fn conflicting_duplicate_target_bodies_are_rejected() {
    assert!(
        fetch(7, vec![(9, "fixture"), (9, "changed")], 10, true)
            .await
            .is_err(),
        "ambiguous duplicate FETCH must not silently choose one body"
    );
}

#[tokio::test]
async fn clean_size_rejection_reuses_the_same_authenticated_socket() {
    let fixture = Fixture::new().await;
    let provider = ImapProvider::new(fixture.config(Security::Tls), credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&fixture.cert).unwrap())
        .unwrap();
    let task = tokio::spawn(async move {
        let mut wire = fixture.accept(false, false).await;
        imap_login(&mut wire, false).await;
        // There is only one accept and login. Reconnection or BODY fetch fails.
        for _ in 0..2 {
            command(
                &mut wire,
                "SELECT \"INBOX\"",
                "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
            )
            .await;
            command(
                &mut wire,
                "UID SEARCH UID 9",
                "* SEARCH 9\r\n$TAG OK found\r\n",
            )
            .await;
            command(
                &mut wire,
                "UID FETCH 9 (UID RFC822.SIZE)",
                "* 1 FETCH (UID 9 RFC822.SIZE 8)\r\n$TAG OK size\r\n",
            )
            .await;
        }
    });
    let reference = MessageRef {
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid: 9,
    };
    for _ in 0..2 {
        assert_eq!(
            provider.fetch(&reference, 7).await.unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
    task.await.unwrap();
}

async fn gmail_mismatched_size(size: u32, eof_response: &str) -> Result<Vec<u8>> {
    let fixture = Fixture::new().await;
    let mut config = fixture.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    let provider = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&fixture.cert).unwrap())
        .unwrap();
    let eof_response = eof_response.to_owned();
    let task = tokio::spawn(async move {
        let mut wire = fixture.accept(false, false).await;
        imap_login(&mut wire, false).await;
        command(
            &mut wire,
            "SELECT \"INBOX\"",
            "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut wire,
            "UID SEARCH UID 9",
            "* SEARCH 9\r\n$TAG OK found\r\n",
        )
        .await;
        command(
            &mut wire,
            "UID FETCH 9 (UID RFC822.SIZE)",
            &format!("* 1 FETCH (UID 9 RFC822.SIZE {size})\r\n$TAG OK size\r\n"),
        )
        .await;
        command(
            &mut wire,
            "UID FETCH 9 (UID BODY.PEEK[]<0.201>)",
            "* 1 FETCH (UID 9 BODY[]<0> {7}\r\nfixture)\r\n$TAG OK body\r\n",
        )
        .await;
        command(
            &mut wire,
            "UID FETCH 9 (UID BODY.PEEK[]<7.1>)",
            &eof_response,
        )
        .await;
    });
    let result = provider
        .fetch(
            &MessageRef {
                account_id: "fixture-account".into(),
                folder: "INBOX".into(),
                uidvalidity: 17,
                uid: 9,
            },
            200,
        )
        .await;
    task.await.unwrap();
    result
}

#[tokio::test]
async fn gmail_size_mismatch_requires_positive_eof_at_exact_returned_offset() {
    assert_eq!(
        gmail_mismatched_size(5, "* 1 FETCH (UID 9 BODY[]<7> {0}\r\n)\r\n$TAG OK eof\r\n")
            .await
            .unwrap(),
        b"fixture"
    );
    assert_eq!(
        gmail_mismatched_size(
            100,
            "* 1 FETCH (UID 9 BODY[]<7> {0}\r\n)\r\n$TAG OK eof\r\n"
        )
        .await
        .unwrap(),
        b"fixture"
    );
    for response in [
        "* 1 FETCH (UID 9 BODY[]<7> {1}\r\nx)\r\n$TAG OK more\r\n",
        "* 1 FETCH (UID 9 BODY[]<0> {0}\r\n)\r\n$TAG OK wrong_offset\r\n",
        "* 1 FETCH (UID 10 BODY[]<7> {0}\r\n)\r\n$TAG OK wrong_uid\r\n",
        "* 1 FETCH (UID 9 BODY[HEADER]<7> {0}\r\n)\r\n$TAG OK wrong_section\r\n",
        "$TAG OK missing_body\r\n",
        "* 1 FETCH (UID 9 BODY[]<7> NIL)\r\n$TAG OK nil_is_not_eof\r\n",
        "* 1 FETCH (UID 9 BODY[]<7> {0}\r\n)\r\n$TAG NO rejected\r\n",
        "* 1 FETCH (UID 9 BODY[]<7> {0}\r\n)\r\n* 1 FETCH (UID 9 BODY[]<7> {1}\r\nx)\r\n$TAG OK conflict\r\n",
    ] {
        assert!(gmail_mismatched_size(100, response).await.is_err(),"accepted invalid EOF evidence");
    }
}

#[tokio::test]
async fn matching_uid_cannot_substitute_a_header_or_nonzero_body_offset() {
    for section in ["BODY[HEADER]<0>", "BODY[]<3>"] {
        assert!(fetch_section(7, vec![(9, "fixture")], 10, true, section)
            .await
            .is_err());
    }
}
