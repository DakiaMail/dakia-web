use super::*;
use std::time::Duration;

#[tokio::test]
async fn queue_wait_counts_toward_every_imap_operation_deadline() {
    let endpoint = Endpoint {
        host: "must-not-connect.invalid".into(),
        port: 993,
        security: Security::Tls,
    };
    let config = Config {
        account_id: "owned".into(),
        primary_address: "sender@example.test".into(),
        kind: ProviderKind::Imap,
        imap: endpoint.clone(),
        smtp: endpoint,
        timeout: Duration::from_millis(10),
        full_imap_exposure_confirmed: false,
        folders: FolderMapping::default(),
    };
    let provider = ImapProvider::new(
        config,
        Credentials {
            username: "private-user".into(),
            password: "private-password".into(),
        },
    )
    .unwrap();
    let reference = MessageRef {
        account_id: "owned".into(),
        folder: "INBOX".into(),
        uidvalidity: 1,
        uid: 1,
    };
    let _guard = provider.lane.lock().await;
    let _idle_guard = provider.idle_lane.lock().await;
    // Holding both lanes means no network or credentials can be used. The
    // outer timeout makes the old unbounded lock wait fail deterministically.
    tokio::time::timeout(Duration::from_secs(1), async {
        assert_eq!(
            provider.probe().await.unwrap_err().kind,
            ErrorKind::Retryable
        );
        assert_eq!(
            provider
                .scan("INBOX", &Search::default(), None, 1)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::Retryable
        );
        assert_eq!(
            provider.fetch(&reference, 1024).await.unwrap_err().kind,
            ErrorKind::Retryable
        );
        let error = provider
            .mutate(&reference, Mutation::Archive)
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Retryable);
        // Even a deadline exhausted wholly in queue keeps its reason. The
        // retryable outcome also proves no write reached IMAP.
        assert_eq!(error.message, "IMAP operation deadline exceeded");
        let raw =
            b"From: sender@example.test\r\nMessage-ID: <stable@example.test>\r\n\r\nSynthetic\r\n";
        assert_eq!(
            provider
                .reconcile_sent("<stable@example.test>", raw)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::Uncertain
        );
        assert_eq!(
            provider
                .wait_for_change("INBOX", Duration::from_secs(1))
                .await
                .unwrap_err()
                .kind,
            ErrorKind::Retryable
        );
    })
    .await
    .expect("queue acquisition must be bounded, not just socket work");
}

#[tokio::test]
async fn queued_user_actions_that_never_acquire_a_connection_are_safe_to_retry() {
    let endpoint = Endpoint {
        host: "must-not-connect.invalid".into(),
        port: 993,
        security: Security::Tls,
    };
    let provider = ImapProvider::new(
        Config {
            account_id: "owned".into(),
            primary_address: "fixture@example.test".into(),
            kind: ProviderKind::Imap,
            imap: endpoint.clone(),
            smtp: endpoint,
            timeout: Duration::from_millis(10),
            full_imap_exposure_confirmed: false,
            folders: FolderMapping::default(),
        },
        Credentials {
            username: "fixture".into(),
            password: "fixture-only".into(),
        },
    )
    .unwrap();
    let reference = MessageRef {
        account_id: "owned".into(),
        folder: "INBOX".into(),
        uidvalidity: 1,
        uid: 1,
    };
    // Simulate background work holding the account command lane. No socket can
    // be opened and no provider command can have been written by these actions.
    let _background = provider.lane.lock().await;
    let kinds = tokio::time::timeout(Duration::from_secs(1), async {
        let mut kinds = Vec::new();
        for action in [
            Mutation::Seen(true),
            Mutation::Starred(true),
            Mutation::Archive,
            Mutation::Trash,
        ] {
            kinds.push(provider.mutate(&reference, action).await.unwrap_err().kind);
        }
        kinds
    })
    .await
    .expect("queue contention must remain bounded");
    assert_eq!(
        kinds,
        vec![ErrorKind::Retryable; 4],
        "a pre-write queue timeout must not tell the user their mail action might have happened"
    );
}
