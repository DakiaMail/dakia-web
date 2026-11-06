//! A slow header batch must not trap history on the same uncommitted page.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
use std::{collections::HashSet, time::Duration};

struct Unknown;
#[async_trait::async_trait]
impl MetadataLookup for Unknown {
    async fn identity_known(&self, _: &Identity) -> Result<bool> {
        Ok(false)
    }
    async fn reference_known(&self, _: &MessageRef) -> Result<bool> {
        Ok(false)
    }
}
fn observations(uids: &[u32], headers: bool) -> String {
    let mut out = String::new();
    for uid in uids.iter().filter(|u| **u != 1) {
        let mut fields =
            format!("UID {uid} FLAGS () X-GM-MSGID {uid} X-GM-THRID 99 X-GM-LABELS ()");
        if headers {
            let h = format!("Subject: synthetic {uid}\r\n\r\n");
            fields += &format!(" RFC822.SIZE 100 INTERNALDATE \"29-Oct-2026 00:00:00 +0000\" BODY[HEADER]<0> {{{}}}\r\n{h}", h.len());
        }
        out += &format!("* 1 FETCH ({fields})\r\n");
    }
    out
}

#[tokio::test]
async fn header_deadline_discards_partial_response_then_smaller_batch_finishes_same_history() {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    config.timeout = Duration::from_secs(1);
    let p = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap();
    let task = tokio::spawn(async move {
        let mut header_batch_sizes = Vec::new();
        for _ in 0..2 {
            let (tcp, _) = f.listener.accept().await.unwrap();
            let mut w = tokio::io::BufReader::new(f.acceptor.accept(tcp).await.unwrap());
            imap_login(&mut w, false).await;
            command(
                &mut w,
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 X-GM-EXT-1\r\n$TAG OK done\r\n",
            )
            .await;
            command(
                &mut w,
                "EXAMINE \"INBOX\"",
                "* OK [UIDVALIDITY 17] epoch\r\n$TAG OK done\r\n",
            )
            .await;
            let all = (1..=60)
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            command(
                &mut w,
                "UID SEARCH ALL",
                &format!("* SEARCH {all}\r\n$TAG OK done\r\n"),
            )
            .await;
            loop {
                let request = line(&mut w).await;
                if request.is_empty() {
                    break;
                }
                let (tag, command) = request.trim_end().split_once(' ').unwrap();
                assert!(command.starts_with("UID FETCH "), "read-only FETCH only");
                let uids: Vec<u32> = command
                    .split_whitespace()
                    .nth(2)
                    .unwrap()
                    .split(',')
                    .map(|u| u.parse().unwrap())
                    .collect();
                let headers = command.contains("BODY.PEEK[HEADER]");
                if headers {
                    header_batch_sizes.push(uids.len());
                }
                if headers && uids.len() >= 50 {
                    // Untagged partial output is not a successful wire chunk.
                    reply(&mut w, &observations(&uids[..5], true)).await;
                    assert!(
                        line(&mut w).await.is_empty(),
                        "deadline must discard socket"
                    );
                    break;
                }
                reply(
                    &mut w,
                    &format!("{}{tag} OK done\r\n", observations(&uids, headers)),
                )
                .await;
                if headers && uids.contains(&2) {
                    break;
                }
            }
        }
        header_batch_sizes
    });
    let error = p
        .reconcile("INBOX", None, 200, &Unknown, None)
        .await
        .err()
        .expect("first batch must time out");
    assert_eq!(error.message, "IMAP operation deadline exceeded");
    // Retry is a separate operation on a new connection, not a command replay.
    let page = p
        .reconcile("INBOX", None, 200, &Unknown, None)
        .await
        .unwrap();
    assert!(page.next.is_none());
    assert!(!page.full_coverage);
    let ids: HashSet<_> = page
        .rows
        .iter()
        .map(|r| match r {
            MetadataChange::New(m) => m.reference.uid,
            _ => panic!("unknown rows need full headers"),
        })
        .collect();
    assert_eq!(ids.len(), 59);
    assert_eq!(page.rows.len(), 59);
    assert!(!ids.contains(&1)); // expunged UID is consumed, not invented
    assert!(ids.contains(&2) && ids.contains(&60));
    let sizes = task.await.unwrap();
    assert_eq!(sizes[0], 50);
    assert!(sizes[1..].iter().all(|n| *n <= 25));
}
