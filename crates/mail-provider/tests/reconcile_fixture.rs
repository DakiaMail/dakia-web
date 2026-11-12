//! Scripted real TLS fixtures. Every command is asserted, no real accounts.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
use std::sync::Arc;

struct Known {
    identity: bool,
    reference: bool,
}
#[async_trait::async_trait]
impl MetadataLookup for Known {
    async fn identity_known(&self, _: &Identity) -> Result<bool> {
        Ok(self.identity)
    }
    async fn reference_known(&self, _: &MessageRef) -> Result<bool> {
        Ok(self.reference)
    }
}
const KNOWN: Known = Known {
    identity: true,
    reference: true,
};
const UNKNOWN: Known = Known {
    identity: false,
    reference: false,
};
async fn scripted(
    steps: Vec<(&'static str, String)>,
) -> (Arc<ImapProvider>, tokio::task::JoinHandle<()>) {
    let fixture = Fixture::new().await;
    let mut config = fixture.config(Security::Tls);
    config.kind = ProviderKind::Gmail;
    let provider = Arc::new(
        ImapProvider::new(config, credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&fixture.cert).unwrap())
            .unwrap(),
    );
    let task = tokio::spawn(async move {
        let mut wire = fixture.accept(false, false).await;
        imap_login(&mut wire, false).await;
        for (expected, response) in steps {
            command(&mut wire, expected, &response).await;
        }
    });
    (provider, task)
}
fn ok(data: &str) -> String {
    format!("{data}$TAG OK completed\r\n")
}
fn caps(extra: &str) -> String {
    ok(&format!("* CAPABILITY IMAP4rev1 X-GM-EXT-1 {extra}\r\n"))
}
fn selected(epoch: u32, high: &str) -> String {
    ok(&format!("* OK [UIDVALIDITY {epoch}] epoch\r\n{high}"))
}
fn changed(uid: u32) -> String {
    ok(&format!(
        "* 1 FETCH (UID {uid} FLAGS (\\Seen \\Flagged) X-GM-MSGID 999 X-GM-LABELS (\\Inbox))\r\n"
    ))
}
fn since() -> ModseqCheckpoint {
    ModseqCheckpoint {
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        modseq: 50,
    }
}

#[tokio::test]
async fn known_gmail_identity_reuses_headers_across_folder_refs_without_fabricated_headers() {
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("")),
        ("EXAMINE \"INBOX\"", selected(17, "")),
        ("UID SEARCH ALL", ok("* SEARCH 8\r\n")),
        ("UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS)", changed(8)),
    ])
    .await;
    let page = p.reconcile("INBOX", None, 200, &KNOWN, None).await.unwrap();
    assert!(!page.full_coverage);
    assert!(page.next.is_none());
    assert!(page.checkpoint.is_none());
    assert!(
        matches!(&page.rows[..], [MetadataChange::Mutable(m)] if m.seen && m.starred && m.reference.uid == 8)
    );
    task.await.unwrap();
}

#[tokio::test]
async fn unknown_identity_fetches_real_headers_and_preserves_thread_identity() {
    let header = "From: synthetic@example.test\r\nSubject: fixture\r\n\r\n";
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("")), ("EXAMINE \"INBOX\"", selected(17, "")),
        ("UID SEARCH ALL", ok("* SEARCH 8\r\n")),
        ("UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS)", changed(8)),
        ("UID FETCH 8 (UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID X-GM-THRID X-GM-LABELS BODY.PEEK[HEADER]<0.65536>)", ok(&format!("* 1 FETCH (UID 8 FLAGS () RFC822.SIZE 100 INTERNALDATE \"29-Oct-2026 00:00:00 +0000\" X-GM-MSGID 999 X-GM-THRID 1001 X-GM-LABELS (\\Inbox) BODY[HEADER]<0> {{{}}}\r\n{header})\r\n", header.len()))),
    ]).await;
    let page = p
        .reconcile("INBOX", None, 200, &UNKNOWN, None)
        .await
        .unwrap();
    assert!(
        matches!(&page.rows[..], [MetadataChange::New(m)] if m.headers == header.as_bytes() && m.gmail_thread_id == Some(1001))
    );
    task.await.unwrap();
}

#[tokio::test]
async fn condstore_pages_enumerate_unchanged_members_and_commit_only_initial_high_watermark() {
    let high = "* OK [HIGHESTMODSEQ 90] high\r\n";
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("CONDSTORE")),
        ("EXAMINE \"INBOX\" (CONDSTORE)", selected(17, high)),
        ("UID SEARCH ALL", ok("* SEARCH 2 8\r\n")),
        (
            "UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS) (CHANGEDSINCE 50)",
            changed(8),
        ),
        (
            "EXAMINE \"INBOX\" (CONDSTORE)",
            selected(17, "* OK [HIGHESTMODSEQ 100] high\r\n"),
        ),
        ("UID SEARCH ALL UID 1:7", ok("* SEARCH 2\r\n")),
        (
            "UID FETCH 2 (UID FLAGS X-GM-MSGID X-GM-LABELS) (CHANGEDSINCE 50)",
            ok(""),
        ),
    ])
    .await;
    let first = p
        .reconcile("INBOX", None, 1, &KNOWN, Some(&since()))
        .await
        .unwrap();
    assert!(first.checkpoint.is_none());
    let token = first.next.unwrap();
    assert_eq!(token.modseq, Some(90));
    let last = p
        .reconcile("INBOX", Some(&token), 1, &KNOWN, Some(&since()))
        .await
        .unwrap();
    assert_eq!(last.checkpoint.unwrap().modseq, 90);
    assert!(matches!(&last.rows[..], [MetadataChange::Unchanged(r)] if r.uid == 2));
    task.await.unwrap();
}

#[tokio::test]
async fn nomodseq_and_not_enabled_qresync_fall_back_to_full_mutable_reconciliation() {
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("CONDSTORE QRESYNC ENABLE")),
        ("ENABLE QRESYNC", ok("* ENABLED\r\n")),
        (
            "EXAMINE \"INBOX\" (CONDSTORE)",
            selected(
                17,
                "* OK [HIGHESTMODSEQ 90] high\r\n* OK [NOMODSEQ] unavailable\r\n",
            ),
        ),
        ("UID SEARCH ALL", ok("* SEARCH 8\r\n")),
        ("UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS)", changed(8)),
    ])
    .await;
    let page = p
        .reconcile("INBOX", None, 200, &KNOWN, Some(&since()))
        .await
        .unwrap();
    assert!(page.checkpoint.is_none());
    assert!(matches!(&page.rows[..], [MetadataChange::Mutable(_)]));
    task.await.unwrap();
    assert!(matches!(
        imap_proto::Response::parse(b"* ENABLED QRESYNC\r\n"),
        Ok((_, imap_proto::Response::Capabilities(_)))
    ));
}

#[tokio::test]
async fn rejected_delta_fetch_falls_back_without_treating_partial_output_as_success() {
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("CONDSTORE")),
        (
            "EXAMINE \"INBOX\" (CONDSTORE)",
            selected(17, "* OK [HIGHESTMODSEQ 90] high\r\n"),
        ),
        ("UID SEARCH ALL", ok("* SEARCH 8\r\n")),
        (
            "UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS) (CHANGEDSINCE 50)",
            "$TAG BAD unsupported\r\n".into(),
        ),
        ("UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS)", changed(8)),
    ])
    .await;
    let page = p
        .reconcile("INBOX", None, 200, &KNOWN, Some(&since()))
        .await
        .unwrap();
    assert!(matches!(&page.rows[..], [MetadataChange::Mutable(_)]));
    task.await.unwrap();
}

#[tokio::test]
async fn partial_throttle_does_not_return_checkpoint_or_replay_and_missing_flags_fail_closed() {
    for response in [
        "* 1 FETCH (UID 8 FLAGS () X-GM-MSGID 999 X-GM-LABELS ())\r\n$TAG NO [LIMIT] throttle\r\n",
        "* 1 FETCH (UID 8 X-GM-MSGID 999 X-GM-LABELS ())\r\n$TAG OK done\r\n",
    ] {
        let (p, task) = scripted(vec![
            ("CAPABILITY", caps("")),
            ("EXAMINE \"INBOX\"", selected(17, "")),
            ("UID SEARCH ALL", ok("* SEARCH 8\r\n")),
            (
                "UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS)",
                response.into(),
            ),
        ])
        .await;
        let result = p.reconcile("INBOX", None, 200, &KNOWN, None).await;
        assert!(matches!(result, Err(e) if e.kind == ErrorKind::Retryable));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn enabled_qresync_vanished_is_not_pruning_authority_and_invalid_epoch_falls_back() {
    for (epoch, high) in [
        (17, "* OK [HIGHESTMODSEQ 90] high\r\n"),
        (18, "* OK [HIGHESTMODSEQ 90] high\r\n"),
        (17, ""),
    ] {
        let delta = epoch == 17 && !high.is_empty();
        let (p, task) = scripted(vec![
            ("CAPABILITY", caps("CONDSTORE QRESYNC ENABLE")),
            ("ENABLE QRESYNC", ok("* ENABLED QRESYNC\r\n")),
            (
                "EXAMINE \"INBOX\" (QRESYNC (17 50))",
                selected(epoch, &format!("{high}* VANISHED (EARLIER) 2:7\r\n")),
            ),
            ("UID SEARCH ALL", ok("* SEARCH 8\r\n")),
            (
                if delta {
                    "UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS) (CHANGEDSINCE 50)"
                } else {
                    "UID FETCH 8 (UID FLAGS X-GM-MSGID X-GM-LABELS)"
                },
                changed(8),
            ),
        ])
        .await;
        let page = p
            .reconcile("INBOX", None, 200, &KNOWN, Some(&since()))
            .await
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert!(
            !page.full_coverage,
            "VANISHED does not certify hidden Gmail coverage"
        );
        assert_eq!(page.checkpoint.is_some(), !high.is_empty());
        task.await.unwrap();
    }
}

#[tokio::test]
async fn epoch_change_abandons_continuation_without_searching_or_fetching() {
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("CONDSTORE")),
        (
            "EXAMINE \"INBOX\" (CONDSTORE)",
            selected(18, "* OK [HIGHESTMODSEQ 90] high\r\n"),
        ),
    ])
    .await;
    let token = PageToken {
        graph_next: None,
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        upper_uid: 8,
        before_uid: 8,
        query: Search::default(),
        modseq: Some(60),
    };
    assert!(
        matches!(p.reconcile("INBOX", Some(&token), 200, &KNOWN, Some(&since())).await, Err(e) if e.kind == ErrorKind::StaleReference)
    );
    task.await.unwrap();
}

#[tokio::test]
async fn completed_snapshot_checks_do_not_search_or_fetch_and_fail_closed_on_change() {
    for (caps_extra, response, expected) in [
        (
            "CONDSTORE",
            selected(17, "* OK [HIGHESTMODSEQ 50] high\r\n"),
            true,
        ),
        (
            "CONDSTORE",
            selected(17, "* OK [HIGHESTMODSEQ 51] high\r\n"),
            false,
        ),
        (
            "CONDSTORE",
            selected(18, "* OK [HIGHESTMODSEQ 50] high\r\n"),
            false,
        ),
        (
            "CONDSTORE",
            selected(17, "* OK [HIGHESTMODSEQ 49] high\r\n"),
            false,
        ),
        (
            "CONDSTORE",
            selected(
                17,
                "* OK [HIGHESTMODSEQ 50] high\r\n* OK [NOMODSEQ] disabled\r\n",
            ),
            false,
        ),
        ("CONDSTORE", selected(17, ""), false),
        ("CONDSTORE", "$TAG BAD unsupported\r\n".into(), false),
        ("", String::new(), false),
    ] {
        let mut steps = vec![("CAPABILITY", caps(caps_extra))];
        if !caps_extra.is_empty() {
            steps.push(("EXAMINE \"INBOX\" (CONDSTORE)", response));
        }
        let (p, task) = scripted(steps).await;
        assert_eq!(p.snapshot_unchanged(&since()).await.unwrap(), expected);
        task.await.unwrap();
    }
}

#[tokio::test]
async fn snapshot_check_scope_and_partial_failure_cannot_certify_unchanged() {
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("CONDSTORE")),
        ("EXAMINE \"INBOX\" (CONDSTORE)", "* OK [UIDVALIDITY 17] epoch\r\n* OK [HIGHESTMODSEQ 50] high\r\n$TAG NO [LIMIT] unavailable\r\n".into()),
    ]).await;
    let mut wrong = since();
    wrong.account_id = "other".into();
    assert_eq!(
        p.snapshot_unchanged(&wrong).await.unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    assert!(p.snapshot_unchanged(&since()).await.is_err());
    task.await.unwrap();
}

#[tokio::test]
async fn equal_modseq_omits_empty_delta_fetch_but_still_enumerates_membership() {
    let (p, task) = scripted(vec![
        ("CAPABILITY", caps("CONDSTORE")),
        (
            "EXAMINE \"INBOX\" (CONDSTORE)",
            selected(17, "* OK [HIGHESTMODSEQ 50] high\r\n"),
        ),
        ("UID SEARCH ALL", ok("* SEARCH 2 8\r\n")),
    ])
    .await;
    let page = p
        .reconcile("INBOX", None, 200, &KNOWN, Some(&since()))
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 2);
    assert!(page
        .rows
        .iter()
        .all(|r| matches!(r, MetadataChange::Unchanged(_))));
    assert_eq!(page.checkpoint.unwrap().modseq, 50);
    assert!(
        !page.full_coverage,
        "equal counter must not certify limited Gmail exposure"
    );
    task.await.unwrap();
}
