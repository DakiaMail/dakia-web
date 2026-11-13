mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};

#[tokio::test]
async fn rejected_store_cannot_be_reported_as_success() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        select(&mut w).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1\r\n$TAG OK caps\r\n",
        )
        .await;
        command(
            &mut w,
            "UID STORE 42 +FLAGS.SILENT (\\Seen)",
            "$TAG NO permission denied\r\n",
        )
        .await;
    });
    assert_eq!(
        p.mutate(&reference(), Mutation::Seen(true))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    server.await.unwrap();
}
#[tokio::test]
async fn rejected_list_cannot_be_reported_as_empty_mailbox() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1\r\n$TAG OK caps\r\n",
        )
        .await;
        command(&mut w, "LIST \"\" *", "$TAG NO unavailable\r\n").await;
    });
    assert_eq!(p.probe().await.unwrap_err().kind, ErrorKind::Unsupported);
    server.await.unwrap();
}
#[tokio::test]
async fn search_disconnect_cannot_be_reported_as_zero_results() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "EXAMINE \"INBOX\"",
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
        )
        .await;
        assert!(line(&mut w).await.contains("UID SEARCH ALL"));
        reply(&mut w, "* SEARCH\r\n").await;
    });
    assert_eq!(
        p.scan("INBOX", &Search::default(), None, 10)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Retryable
    );
    server.await.unwrap();
}
#[tokio::test]
async fn copy_succeeded_but_cleanup_rejected_is_uncertain_not_retryable() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        select(&mut w).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 UIDPLUS\r\n$TAG OK caps\r\n",
        )
        .await;
        command(
            &mut w,
            "UID COPY 42 \"Archive\"",
            "$TAG OK [COPYUID 18 42 55] copied\r\n",
        )
        .await;
        command(
            &mut w,
            "UID STORE 42 +FLAGS.SILENT (\\Deleted)",
            "$TAG NO permission denied\r\n",
        )
        .await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.mutate(&reference(), Mutation::Archive)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Uncertain
    );
    server.await.unwrap();
}

fn provider(f: &Fixture, kind: ProviderKind, security: Security) -> ImapProvider {
    let mut config = f.config(security);
    config.kind = kind;
    ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap()
}
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
#[tokio::test]
async fn idle_uses_a_dedicated_connection_and_reports_change() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 IDLE\r\n$TAG OK caps\r\n",
        )
        .await;
        command(
            &mut w,
            "EXAMINE \"INBOX\"",
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
        )
        .await;
        let idle = line(&mut w).await;
        let (tag, cmd) = idle.trim_end().split_once(' ').unwrap();
        assert_eq!(cmd, "IDLE");
        reply(&mut w, "+ idling\r\n* 2 EXISTS\r\n").await;
        assert_eq!(line(&mut w).await, "DONE\r\n");
        reply(&mut w, &format!("{tag} OK done\r\n")).await;
    });
    assert!(p
        .wait_for_change("INBOX", std::time::Duration::from_secs(1))
        .await
        .unwrap());
    server.await.unwrap();
}
#[tokio::test]
async fn deadline_closes_socket_and_next_probe_reconnects() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.timeout = std::time::Duration::from_millis(300);
    let p = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = f.listener.accept().await.unwrap();
        let mut w = tokio::io::BufReader::new(f.acceptor.accept(tcp).await.unwrap());
        imap_login(&mut w, false).await;
        assert!(line(&mut w).await.contains("CAPABILITY"));
        assert_eq!(line(&mut w).await, "");
        let (tcp, _) = f.listener.accept().await.unwrap();
        let mut w = tokio::io::BufReader::new(f.acceptor.accept(tcp).await.unwrap());
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1\r\n$TAG OK caps\r\n",
        )
        .await;
        command(
            &mut w,
            "LIST \"\" *",
            "* LIST () \"/\" \"INBOX\"\r\n$TAG OK list\r\n",
        )
        .await;
    });
    assert_eq!(p.probe().await.unwrap_err().kind, ErrorKind::Retryable);
    assert_eq!(p.probe().await.unwrap().folders[0].name, "INBOX");
    server.await.unwrap();
}
#[tokio::test]
async fn gmail_ids_labels_thread_headers_and_limited_coverage() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Gmail, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "EXAMINE \"INBOX\"",
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH ALL",
            "* SEARCH 42\r\n$TAG OK search\r\n",
        )
        .await;
        let headers = "Subject: Gmail fixture\r\nDate: Sat, 1 Jan 2000 00:00:00 +0000\r\nMessage-ID: <same@example.test>\r\n\r\n";
        command(&mut w,"UID FETCH 42 (UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID X-GM-THRID X-GM-LABELS BODY.PEEK[HEADER]<0.65536>)",&format!("* 1 FETCH (UID 42 FLAGS (\\Flagged) RFC822.SIZE 100 INTERNALDATE \"28-Oct-2026 20:30:00 +0200\" X-GM-MSGID 1278455344230334865 X-GM-THRID 1266894439832287888 X-GM-LABELS (\\Inbox \"Project label\") BODY[HEADER]<0> {{{}}}\r\n{})\r\n$TAG OK fetch\r\n",headers.len(),headers)).await;
    });
    let page = p.scan("INBOX", &Search::default(), None, 10).await.unwrap();
    server.await.unwrap();
    assert!(!page.full_coverage);
    assert_eq!(page.rows.len(), 1);
    let row = &page.rows[0];
    assert_eq!(
        row.identity,
        Identity::Gmail {
            account_id: "fixture-account".into(),
            message_id: 1278455344230334865
        }
    );
    assert_eq!(row.gmail_thread_id, Some(1266894439832287888));
    assert_eq!(row.internal_date, "28-Oct-2026 20:30:00 +0200");
    assert_eq!(row.labels, vec!["\\Inbox", "Project label"]);
    assert!(!row.seen);
    assert!(row.starred);
}
#[tokio::test]
async fn gmail_archive_uses_all_mail_identity_and_checks_inbox_membership() {
    for move_capability in ["", " MOVE UIDPLUS"] {
        for (readback, inbox_present, succeeds) in [
            ("* 1 FETCH (UID 55 X-GM-MSGID 999 X-GM-LABELS (\\Important Project))\r\n$TAG OK fetch\r\n", false, true),
            ("* 1 FETCH (UID 55 X-GM-MSGID 999 X-GM-LABELS (\\Inbox))\r\n$TAG OK fetch\r\n", false, false),
            ("* 1 FETCH (UID 55 X-GM-MSGID 999 X-GM-LABELS ())\r\n$TAG OK fetch\r\n", true, false),
            ("* 1 FETCH (UID 55 X-GM-MSGID 999 FLAGS ())\r\n$TAG OK fetch\r\n", false, false),
            ("* 1 FETCH (UID 56 X-GM-MSGID 999 X-GM-LABELS ())\r\n$TAG OK fetch\r\n", false, false),
            ("* 1 FETCH (UID 55 X-GM-MSGID 998 X-GM-LABELS ())\r\n$TAG OK fetch\r\n", false, false),
            ("$TAG OK fetch\r\n", false, false),
        ] {
            let f = Fixture::new().await;
            let p = provider(&f, ProviderKind::Gmail, Security::Tls);
            let server = tokio::spawn(async move {
                let mut w = f.accept(false, false).await;
                imap_login(&mut w, false).await;
                select(&mut w).await;
                command(&mut w, "CAPABILITY", &format!("* CAPABILITY IMAP4rev1 X-GM-EXT-1{move_capability}\r\n$TAG OK caps\r\n")).await;
                gmail_archive_reference(&mut w).await;
                command(&mut w, "UID STORE 55 -X-GM-LABELS (\"\\\\Inbox\")", "$TAG OK stored\r\n").await;
                command(&mut w, "UID FETCH 55 (UID X-GM-MSGID X-GM-LABELS)", readback).await;
                if succeeds || inbox_present {
                    command(&mut w, "EXAMINE \"INBOX\"", "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK examined\r\n").await;
                    command(&mut w, "UID SEARCH X-GM-MSGID 999", if inbox_present {"* SEARCH 42\r\n$TAG OK found\r\n"} else {"* SEARCH\r\n$TAG OK gone\r\n"}).await;
                }
                assert_eq!(line(&mut w).await, "");
            });
            let result = p.mutate(&reference(), Mutation::Archive).await;
            if succeeds {
                let result = result.unwrap();
                assert!(result.destination.is_none(), "archive must preserve other labels");
                let dest=result.destination_reference.unwrap();
                assert_eq!((dest.folder.as_str(),dest.uidvalidity,dest.uid),("Archive",18,55));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Uncertain);
            }
            drop(p);
            server.await.unwrap();
        }
    }
}
#[tokio::test]
async fn gmail_archive_rejects_wrong_all_mail_identity_before_writing() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Gmail, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        select(&mut w).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 X-GM-EXT-1 MOVE\r\n$TAG OK caps\r\n",
        )
        .await;
        command(
            &mut w,
            "UID FETCH 42 (UID X-GM-MSGID)",
            "* 1 FETCH (UID 42 X-GM-MSGID 999)\r\n$TAG OK fetch\r\n",
        )
        .await;
        command(
            &mut w,
            "SELECT \"Archive\"",
            "* OK [UIDVALIDITY 18] epoch\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH X-GM-MSGID 999",
            "* SEARCH 55\r\n$TAG OK found\r\n",
        )
        .await;
        command(
            &mut w,
            "UID FETCH 55 (UID X-GM-MSGID)",
            "* 1 FETCH (UID 55 X-GM-MSGID 998)\r\n$TAG OK fetch\r\n",
        )
        .await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.mutate(&reference(), Mutation::Archive)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
    drop(p);
    server.await.unwrap();
}
#[tokio::test]
async fn safe_uidplus_move_does_not_expunge_unrelated_deleted_messages() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        select(&mut w).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 UIDPLUS\r\n$TAG OK caps\r\n",
        )
        .await;
        command(
            &mut w,
            "UID COPY 42 \"Archive\"",
            "$TAG OK [COPYUID 18 42 55] copied\r\n",
        )
        .await;
        command(
            &mut w,
            "UID STORE 42 +FLAGS.SILENT (\\Deleted)",
            "$TAG OK stored\r\n",
        )
        .await;
        command(
            &mut w,
            "UID EXPUNGE 42",
            "* 1 EXPUNGE\r\n$TAG OK expunged\r\n",
        )
        .await;
        assert_eq!(line(&mut w).await, "");
    });
    let result = p.mutate(&reference(), Mutation::Archive).await.unwrap();
    drop(p);
    server.await.unwrap();
    assert_eq!(result.destination_reference.unwrap().uid, 55);
}
#[tokio::test]
async fn no_move_or_uidplus_is_rejected_before_copy_or_delete() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        select(&mut w).await;
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1\r\n$TAG OK caps\r\n",
        )
        .await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.mutate(&reference(), Mutation::Trash)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    server.await.unwrap();
}
#[tokio::test]
async fn stale_uidvalidity_stops_before_fetch() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::StartTls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(true, false).await;
        imap_login(&mut w, true).await;
        command(
            &mut w,
            "SELECT \"INBOX\"",
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 99] reset\r\n$TAG OK selected\r\n",
        )
        .await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.fetch(&reference(), 1024).await.unwrap_err().kind,
        ErrorKind::StaleReference
    );
    server.await.unwrap();
}
#[tokio::test]
async fn invalid_tls_never_sends_login() {
    let f = Fixture::new().await;
    let p = ImapProvider::new(f.config(Security::Tls), credentials()).unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = f.listener.accept().await.unwrap();
        assert!(f.acceptor.accept(tcp).await.is_err());
    });
    assert!(p.probe().await.is_err());
    server.await.unwrap();
}
#[tokio::test]
async fn authentication_rejection_is_typed() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        reply(&mut w, "* OK fixture\r\n").await;
        command(
            &mut w,
            "LOGIN \"fixture\" \"fixture-only-password\"",
            "$TAG NO authentication failed\r\n",
        )
        .await;
    });
    assert_eq!(p.probe().await.unwrap_err().kind, ErrorKind::Authentication);
    server.await.unwrap();
}
#[tokio::test]
async fn cross_account_reference_is_rejected_without_connection() {
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let mut r = reference();
    r.account_id = "another-account".into();
    assert_eq!(
        p.fetch(&r, 1024).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}
#[test]
fn query_escape_rejects_command_injection_and_preserves_fields() {
    let query = Search {
        subject: Some("a\"b\\c".into()),
        from: Some("sender@example.test".into()),
        since: Some("28-Oct-2026".into()),
        seen: Some(false),
        ..Default::default()
    };
    assert_eq!(
        query.imap_query().unwrap(),
        "ALL FROM \"sender@example.test\" SUBJECT \"a\\\"b\\\\c\" SINCE 28-Oct-2026 UNSEEN"
    );
    assert!(Search {
        text: Some("bad\r\nEXPUNGE".into()),
        ..Default::default()
    }
    .imap_query()
    .is_err());
    assert!(Search {
        since: Some("32-Oct-2026".into()),
        ..Default::default()
    }
    .imap_query()
    .is_err());
}
#[tokio::test]
async fn gmail_sent_is_provider_managed_without_append() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    config.smtp.host = "SMTP.GMAIL.COM.".into();
    let p = ImapProvider::new(config, credentials()).unwrap();
    assert_eq!(
        p.reconcile_sent("<x@example.test>", b"unused")
            .await
            .unwrap(),
        SentOutcome::ProviderManaged
    );
}
#[tokio::test]
async fn lost_append_ack_is_filing_unknown_and_never_smtp_submission() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let f = Fixture::new().await;
    let p = provider(&f, ProviderKind::Imap, Security::Tls);
    let raw =
        b"From: sender@example.test\r\nMessage-ID: <sent-fixture@example.test>\r\n\r\nfixture\r\n";
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "LIST \"\" *",
            "* LIST (\\Sent) \"/\" \"Sent\"\r\n$TAG OK list\r\n",
        )
        .await;
        command(
            &mut w,
            "EXAMINE \"Sent\"",
            "* 0 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH HEADER Message-ID \"<sent-fixture@example.test>\"",
            "* SEARCH\r\n$TAG OK search\r\n",
        )
        .await;
        let append = line(&mut w).await;
        assert!(append.contains("APPEND \"Sent\" (\\Seen)"));
        assert!(append.contains(&format!("{{{}}}", raw.len())));
        reply(&mut w, "+ continue\r\n").await;
        let mut bytes = vec![0; raw.len()];
        w.read_exact(&mut bytes).await.unwrap();
        assert_eq!(bytes, raw);
        let mut crlf = String::new();
        w.read_line(&mut crlf).await.unwrap();
        assert_eq!(crlf, "\r\n");
        // Drop after the full literal, before APPEND acknowledgement.
    });
    assert_eq!(
        p.reconcile_sent("<sent-fixture@example.test>", raw)
            .await
            .unwrap(),
        SentOutcome::FilingUnknown
    );
    server.await.unwrap();
}
