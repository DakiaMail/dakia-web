mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
const RAW: &[u8] = b"Message-ID: <stable@example.test>\r\n\r\nfixture";
const LIST: &str =
    "* LIST (\\Sent) \"/\" \"Sent Messages\"\r\n* LIST (\\Sent) \"/\" \"Sent\"\r\n$TAG OK list\r\n";
fn provider(f: &Fixture, explicit: Option<&str>) -> ImapProvider {
    let mut c = f.config(Security::Tls);
    c.folders.sent = explicit.map(str::to_owned);
    ImapProvider::new(c, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap()
}
#[tokio::test]
async fn ambiguous_sent_reconciles_other_view_without_append() {
    let f = Fixture::new().await;
    let p = provider(&f, None);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(&mut w, "LIST \"\" *", LIST).await;
        for (name, ids) in [("Sent", ""), ("Sent Messages", "42")] {
            command(
                &mut w,
                &format!("EXAMINE \"{name}\""),
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK examine\r\n",
            )
            .await;
            command(
                &mut w,
                "UID SEARCH HEADER Message-ID \"<stable@example.test>\"",
                &format!("* SEARCH {ids}\r\n$TAG OK search\r\n"),
            )
            .await;
        }
        let header = "Message-ID: <stable@example.test>\r\n\r\n";
        command(&mut w, "UID FETCH 42 (UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]<0.8192>)", &format!("* 1 FETCH (UID 42 BODY[HEADER.FIELDS (MESSAGE-ID)] {{{}}}\r\n{header})\r\n$TAG OK fetch\r\n", header.len())).await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.reconcile_sent("<stable@example.test>", RAW)
            .await
            .unwrap(),
        SentOutcome::AlreadyPresent
    );
    drop(p);
    server.await.unwrap();
}
#[tokio::test]
async fn failed_candidate_cannot_append_elsewhere() {
    let f = Fixture::new().await;
    let p = provider(&f, None);
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(&mut w, "LIST \"\" *", LIST).await;
        command(&mut w, "EXAMINE \"Sent\"", "$TAG NO unavailable\r\n").await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.reconcile_sent("<stable@example.test>", RAW)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    server.await.unwrap();
}
#[tokio::test]
async fn append_uses_evidence_fallback_and_explicit_preference() {
    use tokio::io::AsyncReadExt;
    for (explicit, counts, expected) in [
        (None, [0, 9], "Sent Messages"),
        (None, [0, 0], "Sent"),
        (Some("Sent"), [0, 9], "Sent"),
    ] {
        let f = Fixture::new().await;
        let p = provider(&f, explicit);
        let server = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            imap_login(&mut w, false).await;
            command(&mut w, "LIST \"\" *", LIST).await;
            for (name, count) in ["Sent", "Sent Messages"].into_iter().zip(counts) {
                command(
                    &mut w,
                    &format!("EXAMINE \"{name}\""),
                    &format!(
                        "* {count} EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK examine\r\n"
                    ),
                )
                .await;
                command(
                    &mut w,
                    "UID SEARCH HEADER Message-ID \"<stable@example.test>\"",
                    "* SEARCH\r\n$TAG OK search\r\n",
                )
                .await;
            }
            let append = line(&mut w).await;
            assert!(append.contains(&format!("APPEND \"{expected}\" (\\Seen)")));
            let tag = append.split_whitespace().next().unwrap();
            reply(&mut w, "+ continue\r\n").await;
            let mut bytes = vec![0; RAW.len() + 2];
            w.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes[..RAW.len()], RAW);
            reply(&mut w, &format!("{tag} OK append\r\n")).await;
        });
        assert_eq!(
            p.reconcile_sent("<stable@example.test>", RAW)
                .await
                .unwrap(),
            SentOutcome::Appended
        );
        server.await.unwrap();
    }
}
