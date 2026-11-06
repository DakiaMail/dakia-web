//! Cold-page acquisition remains bounded and read-only; no real accounts.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};

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
fn sequence(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}
fn observations(uids: &[u32], headers: bool) -> String {
    let mut out = String::new();
    for uid in uids {
        let mut fields = format!(
            "UID {uid} FLAGS (\\Seen) X-GM-MSGID {uid} X-GM-THRID 999 X-GM-LABELS (\\Inbox)"
        );
        if headers {
            let h = format!("From: synthetic@example.test\r\nSubject: fixture {uid}\r\n\r\n");
            fields += &format!(" RFC822.SIZE 100 INTERNALDATE \"29-Oct-2026 00:00:00 +0000\" BODY[HEADER]<0> {{{}}}\r\n{h}", h.len());
        }
        out += &format!("* 1 FETCH ({fields})\r\n");
    }
    out
}

#[tokio::test]
async fn cold_page_skips_repeated_discovery_but_failure_never_returns_checkpoint() {
    for fail_second_chunk in [false, true] {
        let f = Fixture::new().await;
        let mut config = f.config(Security::Tls);
        config.kind = ProviderKind::Gmail;
        let p = ImapProvider::new(config, credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap();
        let server = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            imap_login(&mut w, false).await;
            command(
                &mut w,
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 CONDSTORE X-GM-EXT-1\r\n$TAG OK done\r\n",
            )
            .await;
            command(
                &mut w,
                "EXAMINE \"INBOX\" (CONDSTORE)",
                "* OK [UIDVALIDITY 17] epoch\r\n* OK [HIGHESTMODSEQ 90] high\r\n$TAG OK done\r\n",
            )
            .await;
            let uids: Vec<u32> = (1..=100).rev().collect();
            command(
                &mut w,
                "UID SEARCH ALL",
                &format!(
                    "* SEARCH {}\r\n$TAG OK done\r\n",
                    uids.iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            )
            .await;
            let first = &uids[..50];
            command(
                &mut w,
                &format!(
                    "UID FETCH {} (UID FLAGS X-GM-MSGID X-GM-LABELS)",
                    sequence(first)
                ),
                &format!("{}$TAG OK done\r\n", observations(first, false)),
            )
            .await;
            let full = |uids: &[u32]| {
                format!("UID FETCH {} (UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID X-GM-THRID X-GM-LABELS BODY.PEEK[HEADER]<0.65536>)", sequence(uids))
            };
            command(
                &mut w,
                &full(first),
                &format!("{}$TAG OK done\r\n", observations(first, true)),
            )
            .await;
            // The second chunk must go directly to full metadata, not another
            // mutable discovery. Partial output followed by NO fails closed.
            let second = &uids[50..];
            let terminal = if fail_second_chunk {
                "$TAG NO [LIMIT] throttled\r\n"
            } else {
                "$TAG OK done\r\n"
            };
            command(
                &mut w,
                &full(second),
                &format!("{}{terminal}", observations(second, true)),
            )
            .await;
        });
        let result = p.reconcile("INBOX", None, 100, &Unknown, None).await;
        if fail_second_chunk {
            assert!(matches!(result, Err(e) if e.kind == ErrorKind::Retryable));
        } else {
            let page = result.unwrap();
            assert_eq!(page.rows.len(), 100);
            assert!(page.next.is_none());
            assert!(!page.full_coverage);
            assert_eq!(page.checkpoint.unwrap().modseq, 90);
            assert!(page.rows.iter().all(|m| matches!(m, MetadataChange::New(m) if m.seen && m.gmail_thread_id == Some(999) && !m.headers.is_empty())));
        }
        drop(p);
        server.await.unwrap();
    }
}
