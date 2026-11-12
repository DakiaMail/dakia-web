mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
use tokio::io::BufReader;
fn reference() -> MessageRef {
    MessageRef {
        graph_id: None,
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid: 42,
    }
}
#[tokio::test]
async fn lost_archive_ack_can_be_proven_on_fresh_connection_without_replaying_move() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    let provider = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap();
    let task = tokio::spawn(async move {
        let (socket, _) = f.listener.accept().await.unwrap();
        let mut w = BufReader::new(f.acceptor.accept(socket).await.unwrap());
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "SELECT \"INBOX\"",
            "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH UID 42",
            "* SEARCH 42\r\n$TAG OK found\r\n",
        )
        .await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 MOVE X-GM-EXT-1\r\n$TAG OK caps\r\n",
        )
        .await;
        assert!(line(&mut w).await.ends_with("UID MOVE 42 \"Archive\"\r\n"));
        drop(w); // Provider applied MOVE, but the acknowledgment never arrives.
        let (socket, _) = f.listener.accept().await.unwrap();
        let mut w = BufReader::new(f.acceptor.accept(socket).await.unwrap());
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "EXAMINE \"INBOX\"",
            "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
        )
        .await;
        command(&mut w, "UID SEARCH UID 42", "* SEARCH\r\n$TAG OK gone\r\n").await;
        command(
            &mut w,
            "EXAMINE \"Archive\"",
            "* OK [UIDVALIDITY 18] epoch\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH X-GM-MSGID 999",
            "* SEARCH 101\r\n$TAG OK found\r\n",
        )
        .await;
    });
    let error = provider
        .mutate(&reference(), Mutation::Archive)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Uncertain);
    assert_eq!(
        error.message,
        "IMAP connection closed before tagged completion"
    );
    let identity = Identity::Gmail {
        account_id: "fixture-account".into(),
        message_id: 999,
    };
    let result = provider
        .verify_mutation(&reference(), &identity, &Mutation::Archive)
        .await
        .unwrap()
        .unwrap();
    let dest = result.destination_reference.unwrap();
    assert_eq!(
        (dest.folder.as_str(), dest.uidvalidity, dest.uid),
        ("Archive", 18, 101)
    );
    task.await.unwrap();
}

#[tokio::test]
async fn readback_flags_need_exact_uid_flags_and_epoch_for_both_boolean_values() {
    for (change, flags, expected) in [
        (Mutation::Seen(true), "\\Seen", true),
        (Mutation::Seen(false), "", true),
        (Mutation::Starred(true), "\\Flagged", true),
        (Mutation::Starred(false), "", true),
        (Mutation::Seen(true), "", false),
        (Mutation::Seen(false), "\\Seen", false),
    ] {
        let f = Fixture::new().await;
        let p = ImapProvider::new(f.config(Security::Tls), credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap();
        let task = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            imap_login(&mut w, false).await;
            command(
                &mut w,
                "EXAMINE \"INBOX\"",
                "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
            )
            .await;
            command(
                &mut w,
                "UID FETCH 42 (UID FLAGS)",
                &format!("* 1 FETCH (UID 42 FLAGS ({flags}))\r\n$TAG OK fetched\r\n"),
            )
            .await;
        });
        assert_eq!(
            p.verify_mutation(&reference(), &Identity::Imap(reference()), &change)
                .await
                .unwrap()
                .is_some(),
            expected
        );
        task.await.unwrap();
    }
    for response in [
        "* 1 FETCH (UID 42)\r\n$TAG OK fetched\r\n",
        "* 1 FETCH (UID 99 FLAGS (\\Seen))\r\n$TAG OK fetched\r\n",
        "$TAG OK absent\r\n",
    ] {
        let f = Fixture::new().await;
        let p = ImapProvider::new(f.config(Security::Tls), credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap();
        let task = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            imap_login(&mut w, false).await;
            command(
                &mut w,
                "EXAMINE \"INBOX\"",
                "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK selected\r\n",
            )
            .await;
            command(&mut w, "UID FETCH 42 (UID FLAGS)", response).await;
        });
        assert!(p
            .verify_mutation(
                &reference(),
                &Identity::Imap(reference()),
                &Mutation::Seen(true)
            )
            .await
            .unwrap()
            .is_none());
        task.await.unwrap();
    }
}

#[tokio::test]
async fn absent_archive_destination_or_changed_epoch_never_reports_success() {
    for epoch in [17, 18] {
        let f = Fixture::new().await;
        let mut config = f.config(Security::Tls);
        config.kind = ProviderKind::Gmail;
        let p = ImapProvider::new(config, credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap();
        let task = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            imap_login(&mut w, false).await;
            command(
                &mut w,
                "EXAMINE \"INBOX\"",
                &format!("* OK [UIDVALIDITY {epoch}] epoch\r\n$TAG OK selected\r\n"),
            )
            .await;
            if epoch != 17 {
                return;
            }
            command(&mut w, "UID SEARCH UID 42", "* SEARCH\r\n$TAG OK gone\r\n").await;
            command(
                &mut w,
                "EXAMINE \"Archive\"",
                "* OK [UIDVALIDITY 18] epoch\r\n$TAG OK selected\r\n",
            )
            .await;
            command(
                &mut w,
                "UID SEARCH X-GM-MSGID 999",
                "* SEARCH\r\n$TAG OK absent\r\n",
            )
            .await;
        });
        let result = p
            .verify_mutation(
                &reference(),
                &Identity::Gmail {
                    account_id: "fixture-account".into(),
                    message_id: 999,
                },
                &Mutation::Archive,
            )
            .await;
        if epoch == 17 {
            assert!(result.unwrap().is_none());
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::StaleReference);
        }
        task.await.unwrap();
    }
}
