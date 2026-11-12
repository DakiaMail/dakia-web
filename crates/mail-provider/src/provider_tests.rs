use super::*;
use crate::test_common as common;
use common::*;

#[tokio::test]
async fn cache_lane_waiters_have_a_deadline_without_network_io() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.timeout = Duration::from_millis(20);
    let p = AccountProvider::new(config, credentials()).unwrap();
    let _scan = p.scan_cache.lock().await;
    let _folders = p.folder_cache.lock().await;
    let reference = MessageRef {
        graph_id: None,
        account_id: p.account_id().into(),
        folder: "INBOX".into(),
        uidvalidity: 1,
        uid: 1,
    };
    let q = Search::default();
    let result = tokio::time::timeout(Duration::from_secs(1), async {
        let (scan, folders, mutation, sent) = tokio::join!(
            p.scan("INBOX", &q, None, 50),
            p.folders(),
            p.mutate(&reference, Mutation::Seen(true)),
            p.reconcile_sent("<fixture@example.invalid>", b"fixture")
        );
        assert_eq!(scan.unwrap_err().kind, ErrorKind::Retryable);
        assert_eq!(folders.unwrap_err().kind, ErrorKind::Retryable);
        assert_eq!(mutation.unwrap_err().kind, ErrorKind::Retryable);
        assert_eq!(sent.unwrap_err().kind, ErrorKind::Retryable);
    })
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn actual_tls_scan_is_single_flight_and_failed_mutation_invalidates_cached_pages() {
    let f = Fixture::new().await;
    let config = f.config(Security::Tls);
    let p = AccountProvider {
        imap: ImapProvider::new(config.clone(), credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap(),
        smtp: SmtpProvider::new(config, credentials()).unwrap(),
        folder_cache: Mutex::new(None),
        scan_cache: Mutex::new(ScanCache::default()),
    };
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "EXAMINE \"INBOX\"",
            "* 0 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
        )
        .await;
        command(&mut w, "UID SEARCH ALL", "* SEARCH\r\n$TAG OK search\r\n").await;
    });
    let q = Search::default();
    let (first, second, third) = tokio::join!(
        p.scan("INBOX", &q, None, 50),
        p.scan("INBOX", &q, None, 50),
        p.search("INBOX", &q, None, 50)
    );
    assert!(first.unwrap().rows.is_empty());
    assert!(second.unwrap().rows.is_empty());
    assert!(third.unwrap().rows.is_empty());
    server.await.unwrap();
    // The fixture listener is now closed. A cache hit cannot secretly use a
    // second connection, and the failed mutation must not leave its old page.
    assert!(p.scan("INBOX", &q, None, 50).await.is_ok());
    let reference = MessageRef {
        graph_id: None,
        account_id: p.account_id().into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid: 1,
    };
    assert!(p.mutate(&reference, Mutation::Seen(true)).await.is_err());
    assert!(p.scan("INBOX", &q, None, 50).await.is_err());
}

#[tokio::test]
async fn concurrent_folder_discovery_is_imap_only_and_single_flight() {
    let f = Fixture::new().await;
    let config = f.config(Security::Tls);
    let imap = ImapProvider::new(config.clone(), credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap();
    let p = AccountProvider {
        imap,
        smtp: SmtpProvider::new(config, credentials()).unwrap(),
        folder_cache: Mutex::new(None),
        scan_cache: Mutex::new(ScanCache::default()),
    };
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
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
            "* LIST () \"/\" \"INBOX\"\r\n* LIST (\\Important) \"/\" \"[Gmail]/Important\"\r\n$TAG OK list\r\n",
        )
        .await;
    });
    let (first, second) = tokio::join!(p.folders(), p.folders());
    let first = first.unwrap();
    assert_eq!(first[0].name, "INBOX");
    assert_eq!(first[1].attributes, vec!["Important"]);
    assert_eq!(second.unwrap()[0].name, "INBOX");
    server.await.unwrap();
    assert_eq!(p.folders().await.unwrap()[0].name, "INBOX");
    // No SMTP listener exists. A full account probe cannot succeed here.
    assert!(p.probe().await.is_err());
}

#[tokio::test]
async fn failed_folder_discovery_is_briefly_cached_not_reported_as_empty() {
    let f = Fixture::new().await;
    let config = f.config(Security::Tls);
    let p = AccountProvider {
        imap: ImapProvider::new(config.clone(), credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap(),
        smtp: SmtpProvider::new(config, credentials()).unwrap(),
        folder_cache: Mutex::new(None),
        scan_cache: Mutex::new(ScanCache::default()),
    };
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
    let (first, second) = tokio::join!(p.folders(), p.folders());
    assert_eq!(first.unwrap_err().kind, ErrorKind::Unsupported);
    assert_eq!(second.unwrap_err().kind, ErrorKind::Unsupported);
    server.await.unwrap();
    // Expiry really allows a new attempt; the closed endpoint now fails transport.
    p.folder_cache.lock().await.as_mut().unwrap().0 = Instant::now() - Duration::from_secs(6);
    assert_eq!(p.folders().await.unwrap_err().kind, ErrorKind::Retryable);
    assert_eq!(
        p.folders().await.unwrap_err().message,
        "IMAP reconnect cooling down"
    );
}

#[tokio::test]
async fn interrupted_reads_retry_once_but_auth_deadlines_throttles_and_uncertain_do_not() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (kind, message, calls_expected) in [
        (
            ErrorKind::Retryable,
            "IMAP connection closed before tagged completion",
            2,
        ),
        (ErrorKind::Retryable, "IMAP transport interrupted", 2),
        (ErrorKind::Retryable, "IMAP protocol/transport failed", 2),
        (ErrorKind::Retryable, "IMAP connection failed", 1),
        (ErrorKind::Retryable, "IMAP reconnect cooling down", 1),
        (ErrorKind::Retryable, "IMAP operation deadline exceeded", 1),
        (ErrorKind::Retryable, "IMAP provider throttled", 1),
        (ErrorKind::Authentication, "IMAP authentication failed", 1),
        (ErrorKind::Uncertain, "IMAP transport interrupted", 1),
        (ErrorKind::StaleReference, "message vanished", 1),
    ] {
        let calls = AtomicUsize::new(0);
        let result: Result<()> = retry_read(|| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(error(kind, message)) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), calls_expected);
    }
    let calls = AtomicUsize::new(0);
    let result = retry_read(|| {
        let call = calls.fetch_add(1, Ordering::SeqCst);
        async move {
            if call == 0 {
                Err(error(ErrorKind::Retryable, "IMAP transport interrupted"))
            } else {
                Ok(42)
            }
        }
    })
    .await;
    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn body_read_reconnects_after_eof_and_returns_only_complete_second_response() {
    use tokio::io::BufReader;
    let f = Fixture::new().await;
    let config = f.config(Security::Tls);
    let p = AccountProvider {
        imap: ImapProvider::new(config.clone(), credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap(),
        smtp: SmtpProvider::new(config, credentials()).unwrap(),
        folder_cache: Mutex::new(None),
        scan_cache: Mutex::new(ScanCache::default()),
    };
    let task = tokio::spawn(async move {
        let (socket, _) = f.listener.accept().await.unwrap();
        let mut w = BufReader::new(f.acceptor.accept(socket).await.unwrap());
        imap_login(&mut w, false).await;
        assert!(line(&mut w).await.ends_with("SELECT \"INBOX\"\r\n"));
        drop(w);
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
            "UID FETCH 42 (UID RFC822.SIZE)",
            "* 1 FETCH (UID 42 RFC822.SIZE 7)\r\n$TAG OK size\r\n",
        )
        .await;
        command(
            &mut w,
            "UID FETCH 42 (UID BODY.PEEK[]<0.101>)",
            "* 1 FETCH (UID 42 BODY[]<0> {7}\r\nfixture)\r\n$TAG OK fetched\r\n",
        )
        .await;
    });
    let r = MessageRef {
        graph_id: None,
        account_id: p.account_id().into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid: 42,
    };
    assert_eq!(p.fetch(&r, 100).await.unwrap(), b"fixture");
    task.await.unwrap();
}
