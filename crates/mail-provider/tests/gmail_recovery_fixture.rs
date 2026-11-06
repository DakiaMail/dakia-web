//! Stable-identity recovery uses bounded read-only TLS commands only.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};

async fn resolve(returned_id: u64, rejected_folder: bool) -> Result<Option<Metadata>> {
    let fixture = Fixture::new().await;
    let mut config = fixture.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    let p = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&fixture.cert).unwrap())
        .unwrap();
    let server = tokio::spawn(async move {
        let mut w = fixture.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 X-GM-EXT-1\r\n$TAG OK caps\r\n",
        )
        .await;
        if rejected_folder {
            command(&mut w, "EXAMINE \"All Mail\"", "$TAG NO gone\r\n").await;
        } else {
            command(
                &mut w,
                "EXAMINE \"All Mail\"",
                "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
            )
            .await;
            command(
                &mut w,
                "UID SEARCH X-GM-MSGID 99",
                "* SEARCH\r\n$TAG OK found\r\n",
            )
            .await;
        }
        command(
            &mut w,
            "EXAMINE \"Junk\"",
            "* OK [UIDVALIDITY 18] epoch\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH X-GM-MSGID 99",
            if returned_id == 0 {
                "* SEARCH\r\n$TAG OK absent\r\n"
            } else {
                "* SEARCH 9\r\n$TAG OK found\r\n"
            },
        )
        .await;
        if returned_id > 0 {
            let headers = "Subject: Synthetic recovery\r\n\r\n";
            command(&mut w,"UID FETCH 9 (UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID X-GM-THRID X-GM-LABELS BODY.PEEK[HEADER]<0.65536>)",&format!("* 1 FETCH (UID 9 FLAGS () RFC822.SIZE 100 INTERNALDATE \"28-Oct-2026 20:30:00 +0200\" X-GM-MSGID {returned_id} X-GM-THRID 100 X-GM-LABELS (\\Junk) BODY[HEADER]<0> {{{}}}\r\n{})\r\n$TAG OK metadata\r\n",headers.len(),headers)).await;
        }
    });
    let result = p
        .resolve_gmail_message(99, &["All Mail".into(), "Junk".into()])
        .await;
    server.await.unwrap();
    result
}
#[tokio::test]
async fn stable_gmail_id_locates_a_replacement_uid_and_checks_its_identity() {
    let m = resolve(99, false).await.unwrap().unwrap();
    assert_eq!(m.reference.folder, "Junk");
    assert_eq!(m.reference.uid, 9);
    assert_eq!(m.reference.uidvalidity, 18);
    assert!(matches!(m.identity, Identity::Gmail { message_id: 99, .. }));
}
#[tokio::test]
async fn missing_identity_returns_no_deletion_claim() {
    assert!(resolve(0, false).await.unwrap().is_none())
}
#[tokio::test]
async fn wrong_gmail_identity_is_never_saved_as_a_replacement() {
    assert_eq!(
        resolve(100, false).await.unwrap_err().kind,
        ErrorKind::Retryable
    )
}
#[tokio::test]
async fn discovery_rejects_unbounded_or_injected_scope_before_connecting() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    let p = ImapProvider::new(config, credentials()).unwrap();
    for folders in [
        vec![],
        vec!["Folder".into(); 5],
        vec!["INBOX\r\nUID STORE 1 +FLAGS (\\Deleted)".into()],
    ] {
        assert_eq!(
            p.resolve_gmail_message(99, &folders)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::InvalidInput
        );
    }
}

#[tokio::test]
async fn renamed_or_missing_folder_does_not_hide_a_valid_reference_elsewhere() {
    assert!(resolve(99, true).await.unwrap().is_some());
}
