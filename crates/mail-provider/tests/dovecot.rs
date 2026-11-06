//! Explicit opt-in: runs only against a disposable loopback Dovecot fixture.
#[path = "common/dovecot_bulk.rs"]
mod bulk;
#[path = "common/dovecot_delta.rs"]
mod delta;
use dakia_mail_provider::{imap::ImapProvider, *};
use std::time::Duration;
fn settings(security: Security) -> (Config, Vec<u8>) {
    let key = if security == Security::Tls {
        "DAKIA_FIXTURE_IMAPS_PORT"
    } else {
        "DAKIA_FIXTURE_IMAP_PORT"
    };
    let port = std::env::var(key)
        .expect("fixture port required")
        .parse()
        .unwrap();
    let cert =
        std::fs::read(std::env::var("DAKIA_FIXTURE_CERT").expect("fixture cert path required"))
            .unwrap();
    let endpoint = Endpoint {
        host: "localhost".into(),
        port,
        security,
    };
    (
        Config {
            account_id: "disposable-fixture".into(),
            primary_address: "fixture@example.test".into(),
            kind: ProviderKind::Imap,
            imap: endpoint.clone(),
            smtp: endpoint,
            timeout: Duration::from_secs(5),
            full_imap_exposure_confirmed: false,
            folders: FolderMapping::default(),
        },
        cert,
    )
}
fn credentials() -> Credentials {
    Credentials {
        username: "fixture".into(),
        password: "fixture-only-password".into(),
    }
}
#[tokio::test]
#[ignore = "requires disposable Dovecot fixture; see fixtures/README.md"]
async fn dovecot_archives_sequential_concurrent_and_paginated_refetch() {
    let (mut config, cert) = settings(Security::Tls);
    let mut setup = fixture_session().await;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = format!("Archive-source-{run}");
    let dest = format!("Archive-dest-{run}");
    setup.create(&source).await.unwrap();
    setup.create(&dest).await.unwrap();
    for n in 0..4 {
        let raw = format!("From: fixture@example.test\r\nSubject: archive {n}\r\nMessage-ID: <archive-{run}-{n}@example.test>\r\n\r\nSynthetic only\r\n");
        setup
            .append(&source, None, None, raw.as_bytes())
            .await
            .unwrap();
    }
    config.folders.archive = Some(dest.clone());
    let provider = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_pem(&cert).unwrap())
        .unwrap();
    let first = provider
        .scan(&source, &Search::default(), None, 2)
        .await
        .unwrap();
    let second = provider
        .scan(&source, &Search::default(), first.next.as_ref(), 2)
        .await
        .unwrap();
    provider
        .mutate(&first.rows[0].reference, Mutation::Archive)
        .await
        .unwrap();
    let (x, y) = tokio::join!(
        provider.mutate(&first.rows[1].reference, Mutation::Archive),
        provider.mutate(&second.rows[0].reference, Mutation::Archive)
    );
    x.unwrap();
    y.unwrap();
    for _ in 0..2 {
        let inbox = provider
            .scan(&source, &Search::default(), None, 2)
            .await
            .unwrap();
        assert_eq!(inbox.rows.len(), 1);
        assert_eq!(inbox.rows[0].reference, second.rows[1].reference);
        let archive = provider
            .scan(&dest, &Search::default(), None, 2)
            .await
            .unwrap();
        assert_eq!(archive.rows.len(), 2);
        let rest = provider
            .scan(&dest, &Search::default(), archive.next.as_ref(), 2)
            .await
            .unwrap();
        assert_eq!(rest.rows.len(), 1);
        assert!(rest.next.is_none());
    }
    setup.logout().await.unwrap();
}
async fn fixture_session() -> async_imap::Session<tokio_native_tls::TlsStream<tokio::net::TcpStream>>
{
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
        .login("fixture", "fixture-only-password")
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires disposable Dovecot fixture; see fixtures/README.md"]
async fn dovecot_conversation_header_search_is_read_neutral_and_paginated() {
    let (config, cert) = settings(Security::Tls);
    let provider = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_pem(&cert).unwrap())
        .unwrap();
    let mut setup = fixture_session().await;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let folder = format!("Dakia-thread-{run}");
    setup.create(&folder).await.unwrap();
    for header in [
        format!("Message-ID: <root-{run}@example.test>\r\n"),
        format!("Message-ID: <reply-{run}@example.test>\r\nIn-Reply-To: <root-{run}@example.test>\r\n"),
        format!("Message-ID: <last-{run}@example.test>\r\nReferences: <root-{run}@example.test> <reply-{run}@example.test>\r\n"),
        format!("Message-ID: <unrelated-{run}@example.test>\r\n"),
    ] {
        let raw = format!("From: fixture@example.test\r\nTo: friend@example.test\r\nSubject: Floor heating\r\n{header}\r\nBody mentions <root-{run}@example.test> but must not match a HEADER search.\r\n");
        setup.append(&folder, None, None, raw.as_bytes()).await.unwrap();
    }
    let query = Search {
        conversation_id: Some(format!("root-{run}@example.test")),
        ..Default::default()
    };
    let first = provider.scan(&folder, &query, None, 2).await.unwrap();
    assert_eq!(first.rows.len(), 2);
    let second = provider
        .scan(&folder, &query, first.next.as_ref(), 2)
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(second.next.is_none());
    for row in first.rows.iter().chain(&second.rows) {
        assert!(!row.seen);
    }
    let after = provider
        .scan(&folder, &Search::default(), None, 10)
        .await
        .unwrap();
    assert_eq!(after.rows.len(), 4);
    assert!(after.rows.iter().all(|m| !m.seen));
    setup.logout().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable Dovecot fixture; see fixtures/README.md"]
async fn dovecot_tls_starttls_search_mutations_and_sent() {
    let (config, cert) = settings(Security::Tls);
    let provider = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_pem(&cert).unwrap())
        .unwrap();
    let mut setup = fixture_session().await;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source = format!("Dakia-fixture-{run}");
    let dest = format!("Dakia-moved-{run}");
    setup.create(&source).await.unwrap();
    setup.create(&dest).await.unwrap();
    for n in 0..3 {
        let raw=format!("From: fixture@example.test\r\nTo: fixture@example.test\r\nMessage-ID: <{run}-{n}@example.test>\r\nSubject: synthetic fixture {n}\r\n\r\nbody {n}\r\n");
        setup
            .append(
                &source,
                None,
                if n == 0 {
                    Some("\"01-Jan-2000 00:00:00 +0000\"")
                } else {
                    None
                },
                raw.as_bytes(),
            )
            .await
            .unwrap();
    }
    let probe = provider.probe().await.unwrap();
    assert!(probe
        .folders
        .iter()
        .any(|f| f.name == source && f.selectable));
    assert!(probe
        .folders
        .iter()
        .any(|f| f.attributes.iter().any(|a| a == "Sent")));
    let historical = Search {
        before: Some("01-Jan-2001".into()),
        subject: Some("synthetic fixture 0".into()),
        ..Default::default()
    };
    let uncached = provider.scan(&source, &historical, None, 10).await.unwrap();
    assert_eq!(uncached.rows.len(), 1); // provider search, no local cache exists
    let page = provider
        .scan(&source, &Search::default(), None, 2)
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 2);
    assert!(page.full_coverage);
    let saved_token = serde_json::to_string(page.next.as_ref().unwrap()).unwrap();
    let restored_token: PageToken = serde_json::from_str(&saved_token).unwrap();
    let mut stale_token = restored_token.clone();
    stale_token.uidvalidity += 1;
    assert_eq!(
        provider
            .scan(&source, &Search::default(), Some(&stale_token), 2)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
    assert_eq!(
        provider
            .scan(&source, &historical, Some(&restored_token), 2)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    let second = provider
        .scan(&source, &Search::default(), Some(&restored_token), 2)
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(second.next.is_none());
    let r = page.rows[0].reference.clone();
    assert!(!page.rows[0].seen);
    assert!(
        String::from_utf8(provider.fetch(&r, 1024 * 1024).await.unwrap())
            .unwrap()
            .contains("synthetic fixture")
    );
    let unchanged = provider
        .scan(&source, &Search::default(), None, 10)
        .await
        .unwrap();
    assert!(!unchanged.rows[0].seen);
    provider.mutate(&r, Mutation::Seen(true)).await.unwrap();
    provider.mutate(&r, Mutation::Starred(true)).await.unwrap();
    let query = Search {
        seen: Some(true),
        starred: Some(true),
        ..Default::default()
    };
    assert_eq!(
        provider
            .scan(&source, &query, None, 10)
            .await
            .unwrap()
            .rows
            .len(),
        1
    );
    let moved = provider
        .mutate(&r, Mutation::Move(dest.clone()))
        .await
        .unwrap();
    assert!(moved.destination_reference.is_some());
    assert_eq!(
        provider
            .scan(&source, &Search::default(), None, 10)
            .await
            .unwrap()
            .rows
            .len(),
        2
    );
    let moved_page = provider
        .scan(&dest, &Search::default(), None, 10)
        .await
        .unwrap();
    assert_eq!(moved_page.rows.len(), 1);
    assert!(moved_page.rows[0].seen);
    provider
        .mutate(&moved_page.rows[0].reference, Mutation::Trash)
        .await
        .unwrap();
    assert!(provider
        .scan(&dest, &Search::default(), None, 10)
        .await
        .unwrap()
        .rows
        .is_empty());
    let mut stale = r.clone();
    stale.uidvalidity = stale.uidvalidity.wrapping_add(1);
    assert_eq!(
        provider.fetch(&stale, 1024).await.unwrap_err().kind,
        ErrorKind::StaleReference
    );
    let (config, cert) = settings(Security::StartTls);
    let starttls = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_pem(&cert).unwrap())
        .unwrap();
    assert_eq!(
        starttls
            .scan(&source, &Search::default(), None, 10)
            .await
            .unwrap()
            .rows
            .len(),
        2
    );
    let id = format!("<{run}-sent@example.test>");
    let raw=format!("From: fixture@example.test\r\nTo: fixture@example.test\r\nMessage-ID: {id}\r\nSubject: fixture Sent\r\n\r\nsynthetic only\r\n");
    assert_eq!(
        provider.reconcile_sent(&id, raw.as_bytes()).await.unwrap(),
        SentOutcome::Appended
    );
    assert_eq!(
        provider.reconcile_sent(&id, raw.as_bytes()).await.unwrap(),
        SentOutcome::AlreadyPresent
    );
    // No cleanup EXPUNGE: only disposable fixture volumes contain these messages.
}
