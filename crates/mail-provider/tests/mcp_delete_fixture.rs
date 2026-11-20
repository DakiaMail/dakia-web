mod common;
use common::*;
use dakia_mail_provider::{imap::ImapProvider, *};
fn provider(f: &Fixture) -> ImapProvider {
    ImapProvider::new(f.config(Security::Tls), credentials())
        .unwrap()
        .with_root(native_tls::Certificate::from_der(&f.cert).unwrap())
        .unwrap()
}
fn reference() -> MessageRef {
    MessageRef {
        graph_id: None,
        account_id: "fixture-account".into(),
        folder: "INBOX".into(),
        uidvalidity: 17,
        uid: 42,
    }
}
#[tokio::test]
async fn permanent_delete_uses_uid_expunge_and_verifies_absence() {
    let f = Fixture::new().await;
    let p = provider(&f);
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
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 UIDPLUS\r\n$TAG OK caps\r\n",
        )
        .await;
        command(
            &mut w,
            "UID STORE 42 +FLAGS.SILENT (\\Deleted)",
            "$TAG OK flags\r\n",
        )
        .await;
        command(
            &mut w,
            "UID EXPUNGE 42",
            "* 1 EXPUNGE\r\n$TAG OK removed\r\n",
        )
        .await;
        command(
            &mut w,
            "UID SEARCH UID 42",
            "* SEARCH\r\n$TAG OK absent\r\n",
        )
        .await;
    });
    p.delete_message(&reference()).await.unwrap();
    server.await.unwrap();
}
#[tokio::test]
async fn gmail_non_trash_expunge_is_not_permanent_deletion() {
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
            "LIST \"\" *",
            "* LIST (\\Trash) \"/\" \"Localized Trash\"\r\n$TAG OK listed\r\n",
        )
        .await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.delete_message(&reference()).await.unwrap_err().kind,
        ErrorKind::Unsupported
    );
    server.await.unwrap();
}
#[tokio::test]
async fn missing_uidplus_never_uses_global_expunge() {
    let f = Fixture::new().await;
    let p = provider(&f);
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
        command(
            &mut w,
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1\r\n$TAG OK caps\r\n",
        )
        .await;
        assert_eq!(line(&mut w).await, "");
    });
    assert_eq!(
        p.delete_message(&reference()).await.unwrap_err().kind,
        ErrorKind::Unsupported
    );
    server.await.unwrap();
}
