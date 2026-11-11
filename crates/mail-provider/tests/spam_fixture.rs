//! Disposable TLS protocol fixtures, never owner credentials or mail.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};

async fn spam_case(kind: ProviderKind, mode: &str) -> Result<MutationResult> {
    let f = Fixture::new().await;
    let mut config = f.config(Security::Tls);
    config.kind = kind;
    let provider = ImapProvider::new(config, credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap();
    let mode = mode.to_owned();
    let server = tokio::spawn(async move {
        let mut w = f.accept(false, false).await;
        imap_login(&mut w, false).await;
        command(
            &mut w,
            "SELECT \"INBOX\"",
            "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH UID 42",
            "* SEARCH 42\r\n$TAG OK found\r\n",
        )
        .await;
        let caps = if mode == "uidplus" || mode == "partial" {
            "UIDPLUS"
        } else if mode == "unsafe" {
            ""
        } else {
            "MOVE UIDPLUS"
        };
        command(
            &mut w,
            "CAPABILITY",
            &format!("* CAPABILITY IMAP4rev1 {caps}\r\n$TAG OK caps\r\n"),
        )
        .await;
        let list = match mode.as_str() {
            "missing" => "* LIST () \"/\" \"Spam\"\r\n$TAG OK list\r\n",
            "ambiguous" => "* LIST (\\Junk) \"/\" \"Junk\"\r\n* LIST (\\Junk) \"/\" \"Other Junk\"\r\n$TAG OK list\r\n",
            _ => "* LIST (\\Junk) \"/\" \"Courrier indésirable\"\r\n$TAG OK list\r\n",
        };
        command(&mut w, "LIST \"\" *", list).await;
        if matches!(mode.as_str(), "missing" | "ambiguous" | "unsafe") {
            return;
        }
        let copy = matches!(mode.as_str(), "uidplus" | "partial");
        let verb = if copy { "COPY" } else { "MOVE" };
        let response = match mode.as_str() {
            "reject" => "$TAG NO denied\r\n",
            "no_copyuid" => "$TAG OK moved\r\n",
            _ => "$TAG OK [COPYUID 18 42 55] moved\r\n",
        };
        command(
            &mut w,
            &format!("UID {verb} 42 \"Courrier indésirable\""),
            response,
        )
        .await;
        if mode == "reject" {
            return;
        }
        if copy {
            command(
                &mut w,
                "UID STORE 42 +FLAGS.SILENT (\\Deleted)",
                if mode == "partial" {
                    "$TAG NO denied\r\n"
                } else {
                    "$TAG OK stored\r\n"
                },
            )
            .await;
            if mode == "partial" {
                return;
            }
            command(&mut w, "UID EXPUNGE 42", "$TAG OK expunged\r\n").await;
        }
        command(
            &mut w,
            "UID SEARCH UID 42",
            if mode == "source_present" {
                "* SEARCH 42\r\n$TAG OK found\r\n"
            } else {
                "* SEARCH\r\n$TAG OK found\r\n"
            },
        )
        .await;
        if matches!(mode.as_str(), "source_present" | "no_copyuid") {
            return;
        }
        command(
            &mut w,
            "SELECT \"Courrier indésirable\"",
            if mode == "epoch" {
                "* OK [UIDVALIDITY 19] valid\r\n$TAG OK selected\r\n"
            } else {
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 18] valid\r\n$TAG OK selected\r\n"
            },
        )
        .await;
        if mode == "epoch" {
            return;
        }
        command(
            &mut w,
            "UID SEARCH UID 55",
            if mode == "dest_missing" {
                "* SEARCH\r\n$TAG OK found\r\n"
            } else {
                "* SEARCH 55\r\n$TAG OK found\r\n"
            },
        )
        .await;
        if mode == "dest_missing" {
            return;
        }
        command(
            &mut w,
            "UID SEARCH UID 55",
            "* SEARCH 55\r\n$TAG OK found\r\n",
        )
        .await;
    });
    let result = provider
        .mutate(
            &MessageRef {
                graph_id: None, account_id: "fixture-account".into(),
                folder: "INBOX".into(),
                uidvalidity: 17,
                uid: 42,
            },
            Mutation::Spam,
        )
        .await;
    server.await.unwrap();
    result
}

#[tokio::test]
async fn spam_gmail_and_imap_move_and_uidplus_verify_localized_junk() {
    for kind in [ProviderKind::Gmail, ProviderKind::Imap] {
        for mode in ["move", "uidplus"] {
            let result = spam_case(kind, mode).await.unwrap();
            assert_eq!(result.destination.as_deref(), Some("Courrier indésirable"));
            let r = result.destination_reference.unwrap();
            assert_eq!(
                (r.account_id.as_str(), r.uidvalidity, r.uid),
                ("fixture-account", 18, 55)
            );
        }
    }
}
#[tokio::test]
async fn spam_missing_ambiguous_unsafe_or_rejected_is_not_applied() {
    for mode in ["missing", "ambiguous", "unsafe", "reject"] {
        assert_eq!(
            spam_case(ProviderKind::Imap, mode).await.unwrap_err().kind,
            ErrorKind::Unsupported,
            "{mode}"
        );
    }
}
#[tokio::test]
async fn spam_partial_and_failed_readback_are_uncertain_and_never_replayed() {
    for mode in [
        "partial",
        "source_present",
        "no_copyuid",
        "epoch",
        "dest_missing",
    ] {
        assert_eq!(
            spam_case(ProviderKind::Gmail, mode).await.unwrap_err().kind,
            ErrorKind::Uncertain,
            "{mode}"
        );
    }
}
#[tokio::test]
async fn spam_cross_account_reference_rejected_before_connect() {
    let f = Fixture::new().await;
    let provider = ImapProvider::new(f.config(Security::Tls), credentials()).unwrap();
    let result = provider
        .mutate(
            &MessageRef {
                graph_id: None, account_id: "another-account".into(),
                folder: "INBOX".into(),
                uidvalidity: 17,
                uid: 42,
            },
            Mutation::Spam,
        )
        .await;
    assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidInput);
}
