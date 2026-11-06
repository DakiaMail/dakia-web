//! Measure the actual TLS IMAP transport, not just the provider trait boundary.
mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};

#[tokio::test]
async fn cleanup_moves_reuse_one_tls_login_with_bounded_exact_uid_commands() {
    for count in [48_u32, 200] {
        let f = Fixture::new().await;
        let provider = ImapProvider::new(f.config(Security::Tls), credentials())
            .unwrap()
            .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
            .unwrap();
        let server = tokio::spawn(async move {
            // A reconnect would have no accepting server and fail the test.
            let mut wire = f.accept(false, false).await;
            imap_login(&mut wire, false).await;
            let mut commands = 1;
            for uid in 1..=count {
                command(
                    &mut wire,
                    "SELECT \"INBOX\"",
                    "* 200 EXISTS\r\n* OK [UIDVALIDITY 17] valid\r\n$TAG OK selected\r\n",
                )
                .await;
                command(
                    &mut wire,
                    &format!("UID SEARCH UID {uid}"),
                    &format!("* SEARCH {uid}\r\n$TAG OK found\r\n"),
                )
                .await;
                if uid == 1 {
                    command(
                        &mut wire,
                        "CAPABILITY",
                        "* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n$TAG OK caps\r\n",
                    )
                    .await;
                }
                command(
                    &mut wire,
                    &format!("UID MOVE {uid} \"Trash\""),
                    &format!("$TAG OK [COPYUID 18 {uid} {}] moved\r\n", 1000 + uid),
                )
                .await;
                commands += 3 + u32::from(uid == 1);
            }
            assert_eq!(line(&mut wire).await, "");
            commands
        });
        for uid in 1..=count {
            let result = provider
                .mutate(
                    &MessageRef {
                        account_id: "fixture-account".into(),
                        folder: "INBOX".into(),
                        uidvalidity: 17,
                        uid,
                    },
                    Mutation::Trash,
                )
                .await
                .unwrap();
            assert_eq!(result.destination.as_deref(), Some("Trash"));
            assert_eq!(result.destination_reference.unwrap().uid, 1000 + uid);
        }
        drop(provider);
        let commands = server.await.unwrap();
        assert_eq!(commands, 2 + 3 * count);
        println!("{count} exact Trash moves: one TLS connection/login, {commands} IMAP commands including identity/epoch guards and COPYUID destination references");
    }
}
