use super::*;

async fn contract(repository: ControlRepository) {
    assert_eq!(repository.auth_counts().await.unwrap().users, 0);
    repository
        .put_setting("local_setup_token", "setup-hash")
        .await
        .unwrap();
    assert_eq!(
        repository
            .claim_owner("invalid", "invalid", "encoded", "wrong")
            .await
            .unwrap(),
        OwnerClaim::InvalidCode
    );
    assert_eq!(repository.auth_counts().await.unwrap().users, 0);

    // Both engines must serialize the first-owner decision, not just enforce a
    // unique username after creating several user records.
    let mut claims = tokio::task::JoinSet::new();
    for n in 0..8 {
        let repository = repository.clone();
        claims.spawn(async move {
            let user = format!("owner-{n}");
            let outcome = repository
                .claim_owner(&user, &user, "encoded-password", "setup-hash")
                .await
                .unwrap();
            (user, outcome)
        });
    }
    let mut owner = None;
    while let Some(result) = claims.join_next().await {
        let (user, outcome) = result.unwrap();
        if outcome == OwnerClaim::Created {
            assert!(owner.replace(user).is_none());
        } else {
            assert_eq!(outcome, OwnerClaim::AlreadyConfigured);
        }
    }
    let owner = owner.expect("exactly one owner");
    let counts = repository.auth_counts().await.unwrap();
    assert_eq!((counts.users, counts.logins), (1, 1));
    assert!(repository.is_owner(&owner).await.unwrap());
    assert!(repository
        .setting("local_setup_token")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repository
            .login(&owner)
            .await
            .unwrap()
            .unwrap()
            .password_hash,
        "encoded-password"
    );

    for attempt in 1..=12 {
        assert_eq!(
            repository
                .record_login_attempt("fixture-user", 1000)
                .await
                .unwrap(),
            attempt
        );
    }
    assert_eq!(
        repository
            .record_login_attempt("fixture-user", 1060)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repository
            .record_login_attempt("different-user", 1060)
            .await
            .unwrap(),
        1
    );

    assert!(repository
        .create_invitation("inv-1", "inv-hash-1", &owner, 1000, 1100)
        .await
        .unwrap());
    assert!(!repository
        .invitation_valid("inv-hash-1", 1100)
        .await
        .unwrap());
    assert!(!repository
        .revoke_invitation("inv-1", "other-user")
        .await
        .unwrap());
    assert_eq!(
        repository
            .accept_invitation("inv-hash-1", 1001, "guest", &owner, "encoded-guest")
            .await
            .unwrap(),
        InvitationClaim::UsernameTaken
    );
    assert!(repository
        .invitation_valid("inv-hash-1", 1001)
        .await
        .unwrap());
    // A failing insert must roll back invitation consumption as well.
    assert!(repository
        .accept_invitation("inv-hash-1", 1001, &owner, "unused-name", "encoded-guest")
        .await
        .is_err());
    assert!(repository
        .invitation_valid("inv-hash-1", 1001)
        .await
        .unwrap());
    assert_eq!(
        repository
            .accept_invitation("inv-hash-1", 1001, "guest", "guest", "encoded-guest")
            .await
            .unwrap(),
        InvitationClaim::Created
    );
    assert!(!repository.is_owner("guest").await.unwrap());
    assert_eq!(
        repository
            .accept_invitation("inv-hash-1", 1001, "intruder", "intruder", "encoded")
            .await
            .unwrap(),
        InvitationClaim::InvalidInvitation
    );
    assert!(repository
        .create_invitation("denied", "denied", "guest", 1001, 1100)
        .await
        .is_err());

    assert!(repository
        .create_invitation("race", "race-hash", &owner, 1001, 1100)
        .await
        .unwrap());
    let mut guests = tokio::task::JoinSet::new();
    for n in 0..6 {
        let repository = repository.clone();
        guests.spawn(async move {
            repository
                .accept_invitation(
                    "race-hash",
                    1002,
                    &format!("guest-{n}"),
                    &format!("guest-{n}"),
                    "encoded",
                )
                .await
                .unwrap()
        });
    }
    let mut joined = 0;
    while let Some(result) = guests.join_next().await {
        match result.unwrap() {
            InvitationClaim::Created => joined += 1,
            outcome => assert_eq!(outcome, InvitationClaim::InvalidInvitation),
        }
    }
    assert_eq!(joined, 1);
    assert_eq!(repository.auth_counts().await.unwrap().users, 3);
    for n in 0..20 {
        assert!(repository
            .create_invitation(
                &format!("limit-{n}"),
                &format!("limit-hash-{n}"),
                &owner,
                1002,
                1100
            )
            .await
            .unwrap());
    }
    assert!(!repository
        .create_invitation("over-limit", "over-limit", &owner, 1002, 1100)
        .await
        .unwrap());
    assert_eq!(
        repository.invitations(&owner, 1002).await.unwrap().len(),
        20
    );
    assert!(repository
        .invitations("guest", 1002)
        .await
        .unwrap()
        .is_empty());
    assert!(repository
        .create_invitation("after-expiry", "after-expiry", &owner, 1100, 1200)
        .await
        .unwrap());
    assert_eq!(repository.invitations(&owner, 1100).await.unwrap().len(), 1);
    assert!(repository
        .revoke_invitation("after-expiry", &owner)
        .await
        .unwrap());

    repository
        .create_session("session-1", &owner, "csrf-1", 200)
        .await
        .unwrap();
    repository
        .create_session("session-2", "guest", "csrf-2", 200)
        .await
        .unwrap();
    assert_eq!(
        repository
            .session("session-1", 199)
            .await
            .unwrap()
            .unwrap()
            .user_id,
        owner
    );
    assert!(repository
        .session("session-1", 200)
        .await
        .unwrap()
        .is_none());
    let flow = |state: &str, user: &str| MailOAuthFlow {
        state_hash: state.into(),
        session_hash: "session-1".into(),
        user_id: user.into(),
        verifier: "secret-verifier".into(),
        email: "fixture@example.invalid".into(),
        display_name: "Fixture".into(),
        account_id: None,
        expires_at: 180,
    };
    assert!(repository
        .start_gmail_flow(&flow("forged", "guest"), 100)
        .await
        .is_err());
    repository
        .start_gmail_flow(&flow("state-1", &owner), 100)
        .await
        .unwrap();
    assert!(repository
        .consume_gmail_flow("state-1", "session-2", "guest", 101)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repository
            .consume_gmail_flow("state-1", "session-1", &owner, 101)
            .await
            .unwrap()
            .unwrap()
            .verifier,
        "secret-verifier"
    );
    assert!(repository
        .consume_gmail_flow("state-1", "session-1", &owner, 101)
        .await
        .unwrap()
        .is_none());
    repository
        .start_gmail_flow(&flow("state-2", &owner), 101)
        .await
        .unwrap();
    repository
        .start_gmail_flow(&flow("replacement", &owner), 102)
        .await
        .unwrap();
    assert!(repository
        .consume_gmail_flow("state-2", "session-1", &owner, 103)
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .start_microsoft_flow(&flow("forged", "guest"), 100)
        .await
        .is_err());
    repository
        .start_microsoft_flow(&flow("state-1", &owner), 100)
        .await
        .unwrap();
    assert!(repository
        .consume_microsoft_flow("state-1", "session-2", "guest", 101)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repository
            .consume_microsoft_flow("state-1", "session-1", &owner, 101)
            .await
            .unwrap()
            .unwrap()
            .verifier,
        "secret-verifier"
    );
    assert!(repository
        .consume_microsoft_flow("state-1", "session-1", &owner, 101)
        .await
        .unwrap()
        .is_none());
    repository
        .start_microsoft_flow(&flow("state-2", &owner), 101)
        .await
        .unwrap();
    repository
        .start_microsoft_flow(&flow("replacement", &owner), 102)
        .await
        .unwrap();
    assert!(repository
        .consume_microsoft_flow("state-2", "session-1", &owner, 103)
        .await
        .unwrap()
        .is_none());
    repository.revoke_session("session-1").await.unwrap();
    assert!(repository
        .consume_microsoft_flow("replacement", "session-1", &owner, 103)
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .consume_gmail_flow("replacement", "session-1", &owner, 103)
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .session("session-1", 103)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repository
            .session("session-2", 103)
            .await
            .unwrap()
            .unwrap()
            .csrf,
        "csrf-2"
    );

    let mut identities = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let repository = repository.clone();
        identities.spawn(async move {
            repository
                .user("https://identity.example.invalid", "same-subject")
                .await
                .unwrap()
        });
    }
    let mut identity = None;
    while let Some(result) = identities.join_next().await {
        let user = result.unwrap();
        if let Some(expected) = &identity {
            assert_eq!(&user, expected);
        } else {
            identity = Some(user);
        }
    }
    assert_eq!(repository.auth_counts().await.unwrap().users, 4);
    repository
        .start_oidc_flow("oidc-1", "nonce", "verifier", 100, 200)
        .await
        .unwrap();
    assert_eq!(
        repository
            .consume_oidc_flow("oidc-1", 199)
            .await
            .unwrap()
            .unwrap()
            .nonce,
        "nonce"
    );
    assert!(repository
        .consume_oidc_flow("oidc-1", 199)
        .await
        .unwrap()
        .is_none());
    repository
        .start_oidc_flow("expired", "nonce", "verifier", 100, 200)
        .await
        .unwrap();
    assert!(repository
        .consume_oidc_flow("expired", 200)
        .await
        .unwrap()
        .is_none());

    store_control_contract(repository.clone(), &owner).await;
    account_contract(repository, &owner).await;
}

// Account files remain SQLite at this stage. Exercise the actual Store APIs with
// each control implementation, including the existing encryption and backup path.
async fn store_control_contract(repository: ControlRepository, owner: &str) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::from_control(
        dir.path().join("live"),
        [19; 32],
        8 * 1024 * 1024,
        repository.clone(),
    )
    .await
    .unwrap();
    let mut input: dakia_domain::ConnectAccount = serde_json::from_value(serde_json::json!({
        "email":"owner@example.invalid","display_name":"Fixture","provider":"imap",
        "imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit",
        "smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit",
        "username":"owner","password":"fixture-secret"
    }))
    .unwrap();
    let account = store.connect(owner, &input).await.unwrap();
    let scope = store.authorize(owner, &account.id).await.unwrap();
    assert!(store.authorize("guest", &account.id).await.is_err());
    assert_eq!(
        store.credential(&scope).await.unwrap().password,
        input.password
    );
    let original = repository.credential(owner, &account.id).await.unwrap();
    assert!(!original
        .windows(input.password.len())
        .any(|window| window == input.password.as_bytes()));
    input.password = "replacement-secret".into();
    assert_eq!(store.reconnect(&scope, &input).await.unwrap().revision, 2);
    let updated = repository.credential(owner, &account.id).await.unwrap();
    assert_ne!(original, updated);
    let mut invalid = input.clone();
    invalid.email = "different@example.invalid".into();
    assert!(store.reconnect(&scope, &invalid).await.is_err());
    assert_eq!(
        repository.credential(owner, &account.id).await.unwrap(),
        updated
    );
    let backup = dir.path().join("backup");
    store.backup(&backup).await.unwrap();
    let restored = Store::restore(&backup, dir.path().join("restored"), [19; 32])
        .await
        .unwrap();
    let restored_scope = restored.authorize(owner, &account.id).await.unwrap();
    assert_eq!(
        restored.credential(&restored_scope).await.unwrap().password,
        input.password
    );
    assert_eq!(restored.accounts(owner).await.unwrap()[0].revision, 2);
    assert!(!restored.jobs_enabled().await.unwrap());
    assert!(restored
        .control_repository()
        .session("session-2", 100)
        .await
        .unwrap()
        .is_none());
}

async fn account_contract(repository: ControlRepository, owner: &str) {
    let account = dakia_domain::Account {
        id: "account-fixture".into(),
        email: "mail@example.invalid".into(),
        display_name: "Fixture".into(),
        provider: dakia_domain::ProviderKind::Imap,
        state: dakia_domain::AccountState::Connecting,
        revision: 1,
    };
    let sealed = [0, 255, 17, 0, 89];
    repository
        .insert_account(owner, &account, &sealed, "2026-11-10T00:00:00Z")
        .await
        .unwrap();
    assert!(repository.owns_account(owner, &account.id).await.unwrap());
    assert!(!repository.owns_account("guest", &account.id).await.unwrap());
    assert!(repository.account("guest", &account.id).await.is_err());
    assert!(repository.credential("guest", &account.id).await.is_err());
    assert_eq!(
        repository.credential(owner, &account.id).await.unwrap(),
        sealed
    );
    assert!(repository.accounts("guest").await.unwrap().is_empty());
    let mut updates = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let repository = repository.clone();
        let owner = owner.to_owned();
        updates.spawn(async move {
            repository
                .mutate_account(&owner, "account-fixture", None, |account| {
                    account.revision += 1;
                    Ok(())
                })
                .await
                .unwrap()
                .revision
        });
    }
    let mut revisions = std::collections::BTreeSet::new();
    while let Some(result) = updates.join_next().await {
        revisions.insert(result.unwrap());
    }
    assert_eq!(revisions, (2..=13).collect());
    // A rejected shared mutation must not persist either metadata or ciphertext.
    assert!(repository
        .mutate_account(owner, &account.id, Some(b"replacement"), |account| {
            account.revision += 1;
            Err(crate::Error::Invalid)
        })
        .await
        .is_err());
    assert_eq!(
        repository
            .account(owner, &account.id)
            .await
            .unwrap()
            .revision,
        13
    );
    assert_eq!(
        repository.credential(owner, &account.id).await.unwrap(),
        sealed
    );
    repository
        .save_cursor(owner, "cursor", "owner-document", 100, 200)
        .await
        .unwrap();
    repository
        .save_cursor("guest", "guest-cursor", "guest-document", 100, 200)
        .await
        .unwrap();
    assert!(repository
        .cursor("guest", "cursor", 100)
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .cursor(owner, "cursor", 200)
        .await
        .unwrap()
        .is_none());
    repository
        .subscribe(owner, "endpoint", r#"{"endpoint":"fixture"}"#)
        .await
        .unwrap();
    repository
        .expire_subscription("guest", "endpoint")
        .await
        .unwrap();
    assert_eq!(repository.subscriptions(owner).await.unwrap().len(), 1);
    assert!(repository.subscriptions("guest").await.unwrap().is_empty());

    let mut rule: dakia_domain::MailRule = serde_json::from_value(serde_json::json!({
        "id":"rule-fixture","preset":"block_sender","enabled":true,
        "activated_at":"2026-11-10T00:00:00Z","scope":"all_mailboxes",
        "condition":{"kind":"sender_is","email":"blocked@example.invalid"},
        "actions":["trash","suppress_notifications"]
    }))
    .unwrap();
    repository
        .upsert_rule(owner, "blocked@example.invalid", &rule)
        .await
        .unwrap();
    assert!(repository.rules("guest").await.unwrap().is_empty());
    assert!(repository.rule("guest", &rule.id).await.is_err());
    rule.enabled = false;
    assert!(repository.update_rule("guest", &rule).await.is_err());
    repository.update_rule(owner, &rule).await.unwrap();
    assert!(
        !repository
            .rule_by_sender(owner, "blocked@example.invalid")
            .await
            .unwrap()
            .unwrap()
            .enabled
    );

    let mut writers = tokio::task::JoinSet::new();
    for n in 0..8 {
        let (repository, owner, mut proposed) =
            (repository.clone(), owner.to_owned(), rule.clone());
        proposed.id = format!("concurrent-rule-{n}");
        writers.spawn(async move {
            repository
                .upsert_rule(&owner, "blocked@example.invalid", &proposed)
                .await
                .unwrap()
        });
    }
    while let Some(result) = writers.join_next().await {
        let result = result.unwrap();
        assert_eq!(result.id, rule.id);
        assert_eq!(
            repository.rule(owner, &result.id).await.unwrap().id,
            rule.id
        );
    }
    repository
        .backoff_microsoft_subscription(&account.id, "old-secret", "2026-11-15T00:00:00Z", 100)
        .await
        .unwrap();
    repository
        .save_microsoft_subscription(
            &account.id,
            "graph-subscription",
            false,
            "valid-secret",
            "2026-11-15T00:00:00Z",
        )
        .await
        .unwrap();
    // Webhooks cannot wake connecting accounts or authenticate with a stale secret.
    assert!(repository
        .wake_microsoft_subscription(
            false,
            "",
            "graph-subscription",
            "valid-secret",
            "2026-11-14T00:00:00Z"
        )
        .await
        .unwrap()
        .is_none());
    repository
        .mutate_account(owner, &account.id, None, |a| {
            a.state = dakia_domain::AccountState::Ready;
            a.revision += 1;
            Ok(())
        })
        .await
        .unwrap();
    assert!(repository
        .wake_microsoft_subscription(
            false,
            "",
            "graph-subscription",
            "old-secret",
            "2026-11-14T00:00:00Z"
        )
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repository
            .wake_microsoft_subscription(
                false,
                "",
                "graph-subscription",
                "valid-secret",
                "2026-11-14T00:00:00Z"
            )
            .await
            .unwrap()
            .as_deref(),
        Some(account.id.as_str())
    );
    repository
        .backoff_microsoft_subscription(&account.id, "ignored-secret", "ignored-expiration", 200)
        .await
        .unwrap();
    repository
        .save_microsoft_subscription(
            &account.id,
            "graph-subscription",
            true,
            "ignored-secret",
            "2026-11-16T00:00:00Z",
        )
        .await
        .unwrap();
    assert_eq!(
        repository
            .wake_microsoft_subscription(
                true,
                "2026-11-14T00:00:00Z",
                "graph-subscription",
                "valid-secret",
                "2026-11-14T00:00:00Z"
            )
            .await
            .unwrap()
            .as_deref(),
        Some(account.id.as_str())
    );
    assert_eq!(
        repository
            .microsoft_wake_version(&account.id)
            .await
            .unwrap(),
        2
    );
    assert!(repository
        .wake_microsoft_subscription(
            false,
            "",
            "graph-subscription",
            "valid-secret",
            "2026-11-14T00:00:00Z"
        )
        .await
        .unwrap()
        .is_none());
    repository
        .expire_microsoft_subscription(&account.id, "2026-11-14T00:00:00Z")
        .await
        .unwrap();
    assert!(repository
        .microsoft_subscription(&account.id)
        .await
        .unwrap()
        .unwrap()
        .id
        .is_none());
    repository
        .save_microsoft_subscription(
            &account.id,
            "graph-replacement",
            false,
            "replacement-secret",
            "2026-11-16T00:00:00Z",
        )
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("control.sqlite");
    repository.snapshot_sqlite(&snapshot).await.unwrap();
    let restored = ControlRepository::sqlite(crate::pool(&snapshot).await.unwrap());
    assert_eq!(
        restored.account(owner, &account.id).await.unwrap().revision,
        14
    );
    assert_eq!(
        restored.credential(owner, &account.id).await.unwrap(),
        sealed
    );
    assert_eq!(
        restored.login(owner).await.unwrap().unwrap().password_hash,
        "encoded-password"
    );
    assert_eq!(
        restored
            .cursor(owner, "cursor", 100)
            .await
            .unwrap()
            .as_deref(),
        Some("owner-document")
    );
    assert_eq!(restored.subscriptions(owner).await.unwrap().len(), 1);
    assert!(!restored.owns_account("guest", &account.id).await.unwrap());
    assert!(!restored.rule(owner, &rule.id).await.unwrap().enabled);
    assert_eq!(
        restored.microsoft_wake_version(&account.id).await.unwrap(),
        2
    );
    assert_eq!(
        restored
            .microsoft_subscription(&account.id)
            .await
            .unwrap()
            .unwrap()
            .id
            .as_deref(),
        Some("graph-replacement")
    );

    assert!(repository
        .revoke_account("guest", &account.id)
        .await
        .is_err());
    assert!(repository.removed_accounts().await.unwrap().is_empty());
    repository.revoke_account(owner, &account.id).await.unwrap();
    assert!(repository
        .wake_microsoft_subscription(
            false,
            "",
            "graph-replacement",
            "replacement-secret",
            "2026-11-14T00:00:00Z"
        )
        .await
        .unwrap()
        .is_none());
    assert!(!repository.owns_account(owner, &account.id).await.unwrap());
    assert_eq!(repository.removed_accounts().await.unwrap(), [account.id]);
    assert!(repository
        .cursor(owner, "cursor", 100)
        .await
        .unwrap()
        .is_none());
    assert!(repository
        .cursor("guest", "guest-cursor", 100)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn sqlite_control_auth_contract() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [19; 32]).await.unwrap();
    contract(store.control_repository()).await;
}

#[tokio::test]
#[ignore = "requires disposable dakia_control_test database; DAKIA_TEST_CONTROL_POSTGRES_URL"]
async fn postgres_control_auth_contract() {
    let url = std::env::var("DAKIA_TEST_CONTROL_POSTGRES_URL").unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    assert_eq!(
        options.get_database(),
        Some("dakia_control_test"),
        "refuse to reset any other database"
    );
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query("DROP SCHEMA IF EXISTS dakia_control CASCADE")
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    contract(ControlRepository::open_postgres(&url).await.unwrap()).await;
}
