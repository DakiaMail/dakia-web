//! Real disposable provider state, separate IMAP users, no owner mail.
use super::*;

async fn login(
    username: &str,
) -> async_imap::Session<tokio_native_tls::TlsStream<tokio::net::TcpStream>> {
    let (config, cert) = settings(Security::Tls);
    let tls = native_tls::TlsConnector::builder()
        .add_root_certificate(native_tls::Certificate::from_pem(&cert).unwrap())
        .build()
        .unwrap();
    let tcp = tokio::net::TcpStream::connect(("localhost", config.imap.port))
        .await
        .unwrap();
    let stream = tokio_native_tls::TlsConnector::from(tls)
        .connect("localhost", tcp)
        .await
        .unwrap();
    let mut client = async_imap::Client::new(stream);
    client.read_response().await.unwrap();
    client
        .login(username, "fixture-only-password")
        .await
        .unwrap()
}
fn provider(config: Config, username: &str, cert: &[u8]) -> ImapProvider {
    ImapProvider::new(
        config,
        Credentials {
            username: username.into(),
            password: "fixture-only-password".into(),
        },
    )
    .unwrap()
    .with_root(native_tls::Certificate::from_pem(cert).unwrap())
    .unwrap()
}

#[tokio::test]
#[ignore = "requires disposable Dovecot fixture; see fixtures/README.md"]
async fn bulk_two_accounts_read_star_archive_trash_spam_readback_and_reconnect() {
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut fixtures = Vec::new();
    for index in 0..2 {
        let username = format!("bulk-{run}-{index}");
        let (mut config, cert) = settings(Security::Tls);
        config.account_id = username.clone();
        let source = format!("Source-{index}");
        let archive = format!("Archive-{index}");
        config.folders.archive = Some(archive.clone());
        let mut setup = login(&username).await;
        setup.create(&source).await.unwrap();
        setup.create(&archive).await.unwrap();
        for n in 0..3 {
            let raw = format!("From: fixture@example.test\r\nMessage-ID: <bulk-{run}-{index}-{n}@example.test>\r\nReferences: <same-thread-{run}@example.test>\r\nSubject: selected {index}-{n}\r\n\r\nDisposable exact body {index}-{n}\r\n");
            setup
                .append(&source, None, None, raw.as_bytes())
                .await
                .unwrap();
        }
        setup.logout().await.unwrap();
        let p = provider(config.clone(), &username, &cert);
        let mut rows = p
            .scan(&source, &Search::default(), None, 10)
            .await
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 3);
        // Provider pages are newest first; map explicit seeded message numbers
        // to append UIDs before choosing targets and verifying exact bodies.
        rows.sort_by_key(|row| row.reference.uid);
        fixtures.push((config, cert, username, source, archive, p, rows));
    }
    // Identically numbered source UIDs in different accounts must never alias.
    assert_eq!(
        fixtures[0].6[0].reference.uid,
        fixtures[1].6[0].reference.uid
    );
    assert_eq!(
        fixtures[0]
            .5
            .mutate(&fixtures[1].6[0].reference, Mutation::Seen(true))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    for (_, _, _, source, _, p, rows) in &fixtures {
        for mutation in [Mutation::Seen(true), Mutation::Starred(true)] {
            let (a, b) = tokio::join!(
                p.mutate(&rows[0].reference, mutation.clone()),
                p.mutate(&rows[1].reference, mutation)
            );
            a.unwrap();
            b.unwrap();
        }
        let read = p.scan(source, &Search::default(), None, 10).await.unwrap();
        assert_eq!(
            read.rows
                .iter()
                .filter(|row| row.seen && row.starred)
                .count(),
            2
        );
        let sibling = read
            .rows
            .iter()
            .find(|row| row.reference == rows[2].reference)
            .unwrap();
        assert!(!sibling.seen && !sibling.starred);
        for mutation in [Mutation::Seen(false), Mutation::Starred(false)] {
            let (a, b) = tokio::join!(
                p.mutate(&rows[0].reference, mutation.clone()),
                p.mutate(&rows[1].reference, mutation)
            );
            a.unwrap();
            b.unwrap();
        }
        let read = p.scan(source, &Search::default(), None, 10).await.unwrap();
        assert!(read.rows.iter().all(|row| !row.seen && !row.starred));
    }
    // One provider rejects a stale reference. The other account still succeeds.
    let mut stale = fixtures[1].6[0].reference.clone();
    stale.uidvalidity += 1;
    let (ok, failed) = tokio::join!(
        fixtures[0]
            .5
            .mutate(&fixtures[0].6[0].reference, Mutation::Archive),
        fixtures[1].5.mutate(&stale, Mutation::Archive)
    );
    ok.unwrap();
    assert_eq!(failed.unwrap_err().kind, ErrorKind::StaleReference);
    assert_eq!(
        fixtures[1]
            .5
            .scan(&fixtures[1].3, &Search::default(), None, 10)
            .await
            .unwrap()
            .rows
            .len(),
        3
    );
    fixtures[1]
        .5
        .mutate(&fixtures[1].6[0].reference, Mutation::Archive)
        .await
        .unwrap();
    for (index, (config, cert, username, source, archive, p, rows)) in
        fixtures.into_iter().enumerate()
    {
        p.mutate(&rows[1].reference, Mutation::Trash).await.unwrap();
        let archived = p
            .scan(&archive, &Search::default(), None, 10)
            .await
            .unwrap();
        assert_eq!(archived.rows.len(), 1);
        p.mutate(&archived.rows[0].reference, Mutation::Spam)
            .await
            .unwrap();
        // New provider connections, destination scans and PEEK body readback,
        // not just queue acknowledgement or tagged MOVE responses.
        drop(p);
        let p = provider(config, &username, &cert);
        for _ in 0..2 {
            let remaining = p.scan(&source, &Search::default(), None, 10).await.unwrap();
            assert_eq!(remaining.rows.len(), 1);
            assert_eq!(remaining.rows[0].reference, rows[2].reference);
            assert!(!remaining.rows[0].seen && !remaining.rows[0].starred);
            assert!(p
                .scan(&archive, &Search::default(), None, 10)
                .await
                .unwrap()
                .rows
                .is_empty());
            for (folder, number) in [("Junk", 0), ("Trash", 1)] {
                let dest = p.scan(folder, &Search::default(), None, 10).await.unwrap();
                assert_eq!(dest.rows.len(), 1);
                let body = p.fetch(&dest.rows[0].reference, 65536).await.unwrap();
                assert!(String::from_utf8(body)
                    .unwrap()
                    .contains(&format!("Disposable exact body {index}-{number}")));
            }
        }
    }
}
