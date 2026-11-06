//! Real loopback TLS only. No real mail, credentials, or provider mutations.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
use std::{
    sync::{
        atomic::{AtomicU32, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{Mutex, Notify},
};

#[derive(Default)]
struct State {
    uids: Mutex<Vec<u32>>,
    validity: AtomicU32,
    connections: AtomicUsize,
    log: Mutex<Vec<String>>,
    failure: Mutex<Option<&'static str>>,
    interrupted: Notify,
    headers: AtomicUsize,
    delay: AtomicUsize,
}
struct Server {
    state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
    provider: Arc<ImapProvider>,
}
impl Server {
    async fn new(count: u32) -> Self {
        let f = Fixture::new().await;
        let mut config = f.config(Security::Tls);
        config.kind = ProviderKind::Gmail;
        config.timeout = Duration::from_millis(900);
        let provider = Arc::new(
            ImapProvider::new(config, credentials())
                .unwrap()
                .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
                .unwrap(),
        );
        let state = Arc::new(State::default());
        *state.uids.lock().await = (1..=count).collect();
        state.validity.store(17, Ordering::SeqCst);
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                let (tcp, _) = tokio::select! {
                    accepted = f.listener.accept() => accepted.unwrap(),
                    done = clients.join_next(), if !clients.is_empty() => {
                        done.unwrap().unwrap();
                        continue;
                    }
                };
                let acceptor = f.acceptor.clone();
                let state = shared.clone();
                clients.spawn(async move {
                    let mut w = BufReader::new(acceptor.accept(tcp).await.unwrap());
                    state.connections.fetch_add(1, Ordering::SeqCst);
                    imap_login(&mut w, false).await;
                    loop {
                        let input = line(&mut w).await;
                        if input.is_empty() { break; }
                        let (tag, cmd) = input.trim_end().split_once(' ').unwrap();
                        state.log.lock().await.push(cmd.into());
                        tokio::time::sleep(Duration::from_millis(state.delay.load(Ordering::SeqCst) as u64)).await;
                        let mut response = String::new();
                        if cmd.starts_with("EXAMINE ") || cmd.starts_with("SELECT ") {
                            response = format!("* OK [UIDVALIDITY {}] epoch\r\n", state.validity.load(Ordering::SeqCst));
                        } else if let Some(query) = cmd.strip_prefix("UID SEARCH ") {
                            let mut uids = state.uids.lock().await.clone();
                            if let Some(range) = query.strip_prefix("UID ") {
                                let lower: u32 = range.split(':').next().unwrap().parse().unwrap();
                                if range.contains(':') {
                                    // Exercise IMAP's reversed range with an old last UID.
                                    uids.retain(|u| *u >= lower);
                                    if uids.is_empty() { uids.push(lower.saturating_sub(1)); }
                                } else { uids.retain(|u| *u == lower); }
                            }
                            response = format!("* SEARCH {}\r\n", uids.iter().map(u32::to_string).collect::<Vec<_>>().join(" "));
                        } else if let Some(fetch) = cmd.strip_prefix("UID FETCH ") {
                            let failure = state.failure.lock().await.take();
                            if failure == Some("literal") {
                                reply(&mut w, "* 1 FETCH (UID 1 BODY[HEADER]<0> {1000}\r\npartial").await;
                                state.interrupted.notify_one();
                                let mut closed = String::new();
                                match w.read_line(&mut closed).await {
                                    Ok(0) => {},
                                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {},
                                    other => panic!("cancelled socket must close: {other:?}"),
                                }
                                break;
                            }
                            if failure == Some("eof") { break; }
                            if failure == Some("bye") { reply(&mut w, "* BYE closed\r\n").await; break; }
                            if failure == Some("tag") { reply(&mut w, "WRONG OK completed\r\n").await; continue; }
                            let seq = fetch.split(' ').next().unwrap();
                            for uid in seq.split(',').map(|s| s.parse::<u32>().unwrap()) {
                                if !state.uids.lock().await.contains(&uid) { continue; }
                                let mut h = format!("From: fixture@example.test\r\nSubject: disposable {uid}\r\nMessage-ID: <{uid}@example.test>\r\nX-Padding: ");
                                h.push_str(&"x".repeat(state.headers.load(Ordering::SeqCst)));
                                h.push_str("\r\n\r\n");
                                response += &format!("* 1 FETCH (UID {uid} FLAGS () RFC822.SIZE 70000 INTERNALDATE \"29-Oct-2026 00:00:00 +0000\" X-GM-MSGID {} X-GM-THRID {} X-GM-LABELS (\\Inbox) BODY[HEADER]<0> {{{}}}\r\n{h})\r\n", 100000+u64::from(uid), 100000+u64::from(uid), h.len());
                            }
                            if let Some(kind @ ("NO" | "BAD")) = failure {
                                response += &format!("{tag} {kind} [LIMIT] fixture throttle\r\n");
                                reply(&mut w, &response).await;
                                continue;
                            }
                        } else if cmd == "CAPABILITY" {
                            response = "* CAPABILITY IMAP4rev1 X-GM-EXT-1\r\n".into();
                        } else if cmd == "LIST \"\" *" {
                            response = "* LIST () \"/\" \"INBOX\"\r\n".into();
                        } else {
                            // Only the explicitly tested synthetic flag mutation.
                            assert!(cmd.starts_with("UID STORE ") && cmd.ends_with("+FLAGS.SILENT (\\Seen)"), "unexpected fixture command {cmd}");
                        }
                        response += &format!("{tag} OK done\r\n");
                        reply(&mut w, &response).await;
                    }
                });
            }
        });
        Self {
            state,
            task,
            provider,
        }
    }
    async fn finish(self) {
        self.provider.retire();
        self.task.abort();
        assert!(self.task.await.unwrap_err().is_cancelled());
    }
}
fn watermark(uid: u32) -> HeadCheckpoint {
    HeadCheckpoint {
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid,
    }
}

#[tokio::test]
async fn arrivals_mid_backfill_bursts_sparse_empty_max_and_epoch_reset_share_one_connection() {
    let s = Server::new(200).await;
    let first = s
        .provider
        .scan("INBOX", &Search::default(), None, 50)
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 50);
    let token = first.next.unwrap();
    assert_eq!(token.upper_uid, 200);
    s.state.uids.lock().await.extend(201..=260);
    let head = s
        .provider
        .head("INBOX", Some(&watermark(200)))
        .await
        .unwrap();
    assert_eq!(head.rows.len(), 50);
    assert_eq!(head.checkpoint.uid, 250);
    let head = s
        .provider
        .head("INBOX", Some(&head.checkpoint))
        .await
        .unwrap();
    assert_eq!(head.rows.len(), 10);
    assert_eq!(head.checkpoint.uid, 260);
    let empty = s
        .provider
        .head("INBOX", Some(&head.checkpoint))
        .await
        .unwrap();
    assert!(empty.rows.is_empty());
    assert_eq!(empty.checkpoint.uid, 260);
    let next = s
        .provider
        .scan("INBOX", &Search::default(), Some(&token), 200)
        .await
        .unwrap();
    assert!(next.rows.iter().all(|m| m.reference.uid < 151));
    assert_eq!(next.rows.len(), 150);
    let max = s
        .provider
        .head("INBOX", Some(&watermark(u32::MAX)))
        .await
        .unwrap();
    assert!(max.rows.is_empty());
    assert_eq!(max.checkpoint.uid, u32::MAX);
    *s.state.uids.lock().await = vec![2, 999];
    let sparse = s
        .provider
        .head("INBOX", Some(&watermark(260)))
        .await
        .unwrap();
    assert_eq!(sparse.rows[0].reference.uid, 999);
    s.state.validity.store(18, Ordering::SeqCst);
    *s.state.uids.lock().await = vec![1, 2];
    let reset = s
        .provider
        .head("INBOX", Some(&sparse.checkpoint))
        .await
        .unwrap();
    assert_eq!(reset.checkpoint.uidvalidity, 18);
    assert_eq!(reset.checkpoint.uid, 2);
    assert_eq!(reset.rows.len(), 2);
    assert_eq!(s.state.connections.load(Ordering::SeqCst), 1);
    assert_eq!(
        s.provider
            .scan("INBOX", &Search::default(), Some(&token), 50)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
    let mut foreign = watermark(1);
    foreign.account_id = "other".into();
    assert_eq!(
        s.provider
            .head("INBOX", Some(&foreign))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(
        s.provider
            .head("Archive", Some(&watermark(1)))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    s.finish().await;
}

#[tokio::test]
async fn tagged_rejection_partial_throttle_bye_eof_and_wrong_tag_discard_without_replay() {
    for failure in ["NO", "BAD", "bye", "eof", "tag"] {
        let s = Server::new(2).await;
        *s.state.failure.lock().await = Some(failure);
        assert!(s.provider.head("INBOX", None).await.is_err());
        let page = s.provider.head("INBOX", None).await.unwrap();
        assert_eq!(page.rows.len(), 2);
        assert_eq!(s.state.connections.load(Ordering::SeqCst), 2, "{failure}");
        assert_eq!(
            s.state
                .log
                .lock()
                .await
                .iter()
                .filter(|c| c.starts_with("UID FETCH"))
                .count(),
            2,
            "no automatic command replay"
        );
        s.finish().await;
    }
}

#[tokio::test]
async fn canceled_or_timed_out_literal_is_not_reused_and_retirement_closes_cached_session() {
    for cancel in [true, false] {
        let s = Server::new(1).await;
        *s.state.failure.lock().await = Some("literal");
        let provider = s.provider.clone();
        let task = tokio::spawn(async move { provider.head("INBOX", None).await });
        tokio::time::timeout(Duration::from_secs(2), s.state.interrupted.notified())
            .await
            .unwrap();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            assert_eq!(task.await.unwrap().unwrap_err().kind, ErrorKind::Retryable);
        }
        assert_eq!(s.provider.head("INBOX", None).await.unwrap().rows.len(), 1);
        assert_eq!(s.state.connections.load(Ordering::SeqCst), 2);
        s.provider.retire();
        assert_eq!(
            s.provider.head("INBOX", None).await.unwrap_err().kind,
            ErrorKind::Authentication
        );
        assert_eq!(s.state.connections.load(Ordering::SeqCst), 2);
        s.finish().await;
    }
}

#[tokio::test]
async fn bounded_large_headers_resume_without_loss_and_other_accounts_are_independent() {
    let (s, other) = tokio::join!(Server::new(200), Server::new(1));
    s.state.headers.store(50000, Ordering::SeqCst);
    let query = Search::default();
    let (page, independent) = tokio::join!(
        s.provider.scan("INBOX", &query, None, 200),
        other.provider.head("INBOX", None)
    );
    let page = page.unwrap();
    assert_eq!(independent.unwrap().rows.len(), 1);
    assert!(page.rows.len() > 32 && page.rows.len() < 200);
    assert!(page.rows.iter().map(|r| r.headers.len()).sum::<usize>() < 6 * 1024 * 1024);
    let mut count = page.rows.len();
    let mut next = page.next;
    while let Some(token) = next {
        let page = s
            .provider
            .scan("INBOX", &Search::default(), Some(&token), 200)
            .await
            .unwrap();
        count += page.rows.len();
        next = page.next;
    }
    assert_eq!(count, 200);
    assert!(s
        .provider
        .scan("INBOX", &Search::default(), None, 201)
        .await
        .is_err());
    assert_eq!(s.state.connections.load(Ordering::SeqCst), 1);
    s.finish().await;
    other.finish().await;
}

#[tokio::test]
async fn mutation_gets_the_lane_after_one_bounded_page() {
    let s = Server::new(400).await;
    s.state.delay.store(10, Ordering::SeqCst);
    let p = s.provider.clone();
    let scan = tokio::spawn(async move { p.scan("INBOX", &Search::default(), None, 200).await });
    loop {
        if !s.state.log.lock().await.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    let started = Instant::now();
    let r = MessageRef {
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid: 1,
    };
    s.provider.mutate(&r, Mutation::Seen(true)).await.unwrap();
    let delay = started.elapsed();
    assert_eq!(scan.await.unwrap().unwrap().rows.len(), 200);
    assert!(delay < Duration::from_secs(2), "{delay:?}");
    println!(
        "synthetic_mutation_queue_and_service_ms={}",
        delay.as_millis()
    );
    assert_eq!(s.state.connections.load(Ordering::SeqCst), 1);
    s.finish().await;
}
