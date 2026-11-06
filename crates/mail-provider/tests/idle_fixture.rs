mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn idle_timeout_unsupported_disconnect_and_retirement_are_distinct() {
    for mode in ["timeout", "unsupported", "disconnect", "retire"] {
        let f = Fixture::new().await;
        let p = Arc::new(
            ImapProvider::new(f.config(Security::Tls), credentials())
                .unwrap()
                .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
                .unwrap(),
        );
        let (ready, started) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let mut w = f.accept(false, false).await;
            imap_login(&mut w, false).await;
            let caps = if mode == "unsupported" {
                "* CAPABILITY IMAP4rev1\r\n$TAG OK caps\r\n"
            } else {
                "* CAPABILITY IMAP4rev1 IDLE\r\n$TAG OK caps\r\n"
            };
            command(&mut w, "CAPABILITY", caps).await;
            if mode == "unsupported" {
                return;
            }
            command(
                &mut w,
                "EXAMINE \"INBOX\"",
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
            )
            .await;
            let idle = line(&mut w).await;
            let tag = idle.split_once(' ').unwrap().0;
            reply(&mut w, "+ idling\r\n").await;
            let _ = ready.send(());
            if mode == "disconnect" {
                return;
            }
            if mode == "retire" {
                use tokio::io::AsyncBufReadExt;
                let mut tail = String::new();
                match w.read_line(&mut tail).await {
                    Ok(n) => assert_eq!(n, 0),
                    Err(e) => assert!(matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof
                    )),
                }
            } else {
                assert_eq!(line(&mut w).await, "DONE\r\n");
                reply(&mut w, &format!("{tag} OK done\r\n")).await;
            }
        });
        let copy = p.clone();
        let task = tokio::spawn(async move {
            copy.wait_for_change("INBOX", Duration::from_millis(200))
                .await
        });
        if mode == "retire" {
            started.await.unwrap();
            p.retire();
        }
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        match mode {
            "timeout" => assert!(!result.unwrap()),
            "unsupported" => assert_eq!(result.unwrap_err().kind, ErrorKind::Unsupported),
            "disconnect" => assert_eq!(result.unwrap_err().kind, ErrorKind::Retryable),
            "retire" => {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Authentication);
                assert_eq!(
                    p.wait_for_change("INBOX", Duration::from_secs(1))
                        .await
                        .unwrap_err()
                        .kind,
                    ErrorKind::Authentication
                );
            }
            _ => unreachable!(),
        }
        server.await.unwrap();
    }
}
