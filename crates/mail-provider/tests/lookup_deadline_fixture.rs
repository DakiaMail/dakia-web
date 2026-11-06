//! Local evidence latency shares the operation deadline with wire I/O.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
use std::{collections::HashSet, time::Duration};
struct SlowKnown;
#[async_trait::async_trait]
impl MetadataLookup for SlowKnown {
    async fn identity_known(&self, _: &Identity) -> Result<bool> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(true)
    }
    async fn reference_known(&self, _: &MessageRef) -> Result<bool> {
        Ok(false)
    }
}
#[tokio::test]
async fn local_lookup_deadline_reduces_batch_and_resumes_without_skipping() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    let p = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap();
    let task = tokio::spawn(async move {
        for _ in 0..2 {
            let (tcp, _) = f.listener.accept().await.unwrap();
            let mut w = tokio::io::BufReader::new(f.acceptor.accept(tcp).await.unwrap());
            imap_login(&mut w, false).await;
            loop {
                let request = line(&mut w).await;
                if request.is_empty() {
                    break;
                }
                let (tag, cmd) = request.trim_end().split_once(' ').unwrap();
                let response = if cmd == "CAPABILITY" {
                    "* CAPABILITY IMAP4rev1 X-GM-EXT-1\r\n".into()
                } else if cmd == "EXAMINE \"INBOX\"" {
                    "* OK [UIDVALIDITY 17] epoch\r\n".into()
                } else if cmd.starts_with("UID SEARCH ALL") {
                    let upper = cmd
                        .rsplit_once("1:")
                        .map(|(_, n)| n.parse::<u32>().unwrap())
                        .unwrap_or(60);
                    format!(
                        "* SEARCH {}\r\n",
                        (1..=upper)
                            .map(|u| u.to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                } else {
                    assert!(cmd.starts_with("UID FETCH ") && !cmd.contains("BODY"));
                    let uids = cmd.split_whitespace().nth(2).unwrap();
                    uids.split(',')
                        .map(|u| {
                            format!(
                                "* 1 FETCH (UID {u} FLAGS () X-GM-MSGID {u} X-GM-LABELS ())\r\n"
                            )
                        })
                        .collect::<String>()
                };
                reply(&mut w, &format!("{response}{tag} OK done\r\n")).await;
            }
        }
    });
    let first = p.reconcile("INBOX", None, 200, &SlowKnown, None).await;
    assert!(matches!(first, Err(e) if e.message == "IMAP operation deadline exceeded"));
    let mut next = None;
    let mut seen = HashSet::new();
    let mut pages = 0;
    loop {
        let page = p
            .reconcile("INBOX", next.as_ref(), 200, &SlowKnown, None)
            .await
            .unwrap();
        assert!(!page.full_coverage);
        for row in page.rows {
            let MetadataChange::Mutable(m) = row else {
                panic!("known fixture identity")
            };
            assert!(seen.insert(m.reference.uid), "no duplicate committed UID");
        }
        pages += 1;
        next = page.next;
        if next.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 60);
    assert_eq!(pages, 3);
    drop(p);
    task.await.unwrap();
}
