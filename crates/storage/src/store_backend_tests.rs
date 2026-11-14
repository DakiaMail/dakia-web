use super::*;
fn input() -> ConnectAccount {
    serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap()
}
async fn contract(
    application: Option<&str>,
    search_postgres: Option<&str>,
    import_target: Option<&str>,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("source");
    let mut store = Store::open_with_database(&root, [19; 32], 1024 * 1024 * 1024, application)
        .await
        .unwrap();
    let sqlite_search = format!("sqlite:{}", root.join("selected-search.sqlite").display());
    store
        .configure_search_database(Some(search_postgres.unwrap_or(&sqlite_search)))
        .await
        .unwrap();
    // Complete connection warming separately from the online background path.
    store
        .search_database
        .as_ref()
        .unwrap()
        .warm()
        .await
        .unwrap();
    let setup_owner = if application.is_none() {
        let owner = Uuid::new_v4().to_string();
        let control = store.control_repository();
        control
            .put_setting("local_setup_token", "fixture-setup")
            .await
            .unwrap();
        assert_eq!(
            control
                .claim_owner(&owner, "fixture-owner", "fixture-hash", "fixture-setup")
                .await
                .unwrap(),
            crate::control_repository::OwnerClaim::Created
        );
        control
            .create_invitation("before-invite", "before-hash", &owner, 1, 100)
            .await
            .unwrap();
        assert_eq!(
            control
                .accept_invitation(
                    "before-hash",
                    2,
                    &Uuid::new_v4().to_string(),
                    "before-import",
                    "fixture-hash"
                )
                .await
                .unwrap(),
            crate::control_repository::InvitationClaim::Created
        );
        Some(owner)
    } else {
        None
    };
    let user = store
        .user("fixture", &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let guest = store
        .user("fixture", &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let mut input = input();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    assert!(store.authorize(&guest, &account.id).await.is_err());
    assert_eq!(store.credential(&a).await.unwrap().password, input.password);
    if application.is_some() {
        assert!(Store::open_offline_database(
            dir.path().join("blocked-offline"),
            [19; 32],
            application
        )
        .await
        .is_err());
    }
    input.password = "replaced-fixture".into();
    store.reconnect(&a, &input).await.unwrap();
    assert_eq!(store.credential(&a).await.unwrap().password, input.password);
    let m = store
        .upsert_message(&a, message(&a, "matrix-message"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_records(
            &a,
            &[SearchContent {
                message: &m,
                text: "matrix body token",
                state: "ready",
                participants: "Casey cc@example.invalid",
                recipients: "Casey cc@example.invalid",
                attachments: "archive.txt attachment token",
                attachment_details: "[]",
            }],
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let query = CachedSearchFilter {
        text: Some("matrix body"),
        ..Default::default()
    };
    assert_eq!(
        store
            .full_text_ranked_rows(&a, &query, None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.text_coverage(&a).await.unwrap().processed, 1);
    let attachment = store
        .put_attachment(
            &a,
            "archive.txt".into(),
            "text/plain".into(),
            b"durable attachment\0",
        )
        .await
        .unwrap();
    let draft:Draft=serde_json::from_value(serde_json::json!({"id":"matrix-draft","account_id":a.id(),"revision":0,"to":[{"email":"recipient@example.invalid","name":null}],"cc":[],"bcc":[],"subject":"Archived","text":"Body","html":"","in_reply_to":null,"references":[],"attachments":[attachment],"saved_at":""})).unwrap();
    let draft = store.save_draft(&a, draft).await.unwrap();
    let request = SubmitRequest {
        operation_id: "matrix-submit".into(),
        draft_id: draft.id.clone(),
        expected_revision: draft.revision,
        due_at: Some("2099-01-01T00:00:00.000Z".into()),
        timezone: "UTC".into(),
    };
    let submission = store.submit(&a, &request).await.unwrap();
    assert!(matches!(
        store.remove_account(&a, &input.email).await,
        Err(Error::AccountBusy)
    ));
    let backup = dir.path().join("backup");
    store.backup(&backup).await.unwrap();
    let restored = Store::restore(&backup, dir.path().join("restored"), [19; 32])
        .await
        .unwrap();
    let restored_a = restored.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        restored.credential(&restored_a).await.unwrap().password,
        input.password
    );
    assert_eq!(restored.draft(&restored_a, &draft.id).await.unwrap(), draft);
    assert_eq!(
        restored
            .submission(&restored_a, &submission.id)
            .await
            .unwrap(),
        submission
    );
    assert_eq!(
        restored.submit(&restored_a, &request).await.unwrap(),
        submission
    );
    assert_eq!(
        restored
            .attachment(&restored_a, &draft.attachments[0].id)
            .await
            .unwrap()
            .1,
        b"durable attachment\0"
    );
    assert!(!restored.jobs_enabled().await.unwrap());
    assert_eq!(
        restored.text_coverage(&restored_a).await.unwrap().headers,
        0
    );
    assert!(
        Store::restore(&backup, dir.path().join("wrong-key"), [20; 32])
            .await
            .is_err()
    );
    let imported_root = dir.path().join("imported");
    let imported = Store::import_backend(&backup, &imported_root, [19; 32], import_target)
        .await
        .unwrap();
    let imported_a = imported.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        imported.credential(&imported_a).await.unwrap().password,
        input.password
    );
    assert_eq!(
        imported
            .submission(&imported_a, &submission.id)
            .await
            .unwrap(),
        submission
    );
    assert_eq!(
        imported.submit(&imported_a, &request).await.unwrap(),
        submission
    );
    assert_eq!(
        imported
            .attachment(&imported_a, &draft.attachments[0].id)
            .await
            .unwrap()
            .1,
        b"durable attachment\0"
    );
    assert!(!imported.jobs_enabled().await.unwrap());
    if import_target.is_some() {
        assert!(!imported_root.join("control.sqlite").exists());
        let owner = setup_owner.as_ref().unwrap();
        let control = imported.control_repository();
        control
            .create_invitation("after-invite", "after-hash", owner, 1, 100)
            .await
            .unwrap();
        assert_eq!(
            control
                .accept_invitation(
                    "after-hash",
                    2,
                    &Uuid::new_v4().to_string(),
                    "after-import",
                    "fixture-hash"
                )
                .await
                .unwrap(),
            crate::control_repository::InvitationClaim::Created
        );
        assert!(Store::open_with_database(
            dir.path().join("blocked-online"),
            [19; 32],
            1024,
            import_target
        )
        .await
        .is_err());
        assert!(Store::import_backend(
            &backup,
            dir.path().join("refused-populated"),
            [19; 32],
            import_target
        )
        .await
        .is_err());
        assert_eq!(
            imported
                .submission(&imported_a, &submission.id)
                .await
                .unwrap(),
            submission
        );
        let roundtrip = dir.path().join("roundtrip-archive");
        imported.backup(&roundtrip).await.unwrap();
        let final_store =
            Store::import_backend(&roundtrip, dir.path().join("roundtrip"), [19; 32], None)
                .await
                .unwrap();
        let final_a = final_store.authorize(&user, a.id()).await.unwrap();
        assert_eq!(
            final_store
                .submission(&final_a, &submission.id)
                .await
                .unwrap(),
            submission
        );
        assert_eq!(
            final_store.submit(&final_a, &request).await.unwrap(),
            submission
        );
        assert_eq!(
            final_store.credential(&final_a).await.unwrap().password,
            input.password
        );
    }
    let failed_root = dir.path().join("import-wrong-key");
    assert!(Store::import_backend(&backup, &failed_root, [20; 32], None)
        .await
        .is_err());
    assert!(failed_root.join("backend-import-in-progress").exists());
    assert!(Store::open(&failed_root, [19; 32]).await.is_err());
    // Another Store must authorize its own capability even with the same DB URL.
    let reopened = Store::open_with_database(&root, [19; 32], 1024 * 1024 * 1024, application)
        .await
        .unwrap();
    assert!(matches!(
        reopened.message(&a, &m.id).await,
        Err(Error::NotFound)
    ));
    let reopened_a = reopened.authorize(&user, a.id()).await.unwrap();
    assert_eq!(reopened.message(&reopened_a, &m.id).await.unwrap(), m);
    store
        .transition(
            &a,
            &submission.id,
            SubmissionState::Scheduled,
            SubmissionState::Cancelled,
        )
        .await
        .unwrap();
    store.remove_account(&a, &input.email).await.unwrap();
    assert!(store.message(&a, &m.id).await.is_err());
    assert!(reopened.message(&reopened_a, &m.id).await.is_err());
    assert!(store.authorize(&user, a.id()).await.is_err());
    if application.is_some() {
        assert!(!root.join("control.sqlite").exists());
        assert!(!root
            .join("accounts")
            .join(format!("{}.sqlite", a.id()))
            .exists());
    }
}
#[tokio::test]
async fn sqlite_store_portable_account_backup_contract() {
    contract(None, None, None).await;
}
#[tokio::test]
#[ignore = "requires disposable dakia_store_test database; DAKIA_TEST_STORE_POSTGRES_URL"]
async fn four_backend_store_backup_and_search_contract() {
    let url = std::env::var("DAKIA_TEST_STORE_POSTGRES_URL").unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    assert_eq!(
        options.get_database(),
        Some("dakia_store_test"),
        "refuse to reset any other database"
    );
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for schema in ["dakia_mail", "dakia_control", "dakia_search"] {
        sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
    contract(None, None, None).await;
    contract(None, Some(&url), None).await;
    contract(Some(&url), None, None).await;
    contract(Some(&url), Some(&url), None).await;
}

#[tokio::test]
#[ignore = "requires disposable dakia_import_test database; DAKIA_TEST_IMPORT_POSTGRES_URL"]
async fn offline_backend_transfer_roundtrip_contract() {
    let url = std::env::var("DAKIA_TEST_IMPORT_POSTGRES_URL").unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    assert_eq!(
        options.get_database(),
        Some("dakia_import_test"),
        "refuse to reset any other database"
    );
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for schema in ["dakia_mail", "dakia_control", "dakia_search"] {
        sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
    contract(None, None, Some(&url)).await;
    interrupted_import_guard(&url).await;
}
async fn interrupted_import_guard(url: &str) {
    let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    for schema in ["dakia_mail", "dakia_control", "dakia_search"] {
        sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
    let control = crate::control_repository::ControlRepository::open_postgres(url)
        .await
        .unwrap();
    let bootstrap = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA dakia_mail")
        .execute(&bootstrap)
        .await
        .unwrap();
    bootstrap.close().await;
    let native = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.options([("search_path", "dakia_mail,public")]))
        .await
        .unwrap();
    sqlx::migrate!("./migrations/account_postgres")
        .run(&native)
        .await
        .unwrap();
    sqlx::query("CREATE FUNCTION import_failure() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture interrupted import'; END $$").execute(&native).await.unwrap();
    sqlx::query("CREATE TRIGGER import_failure BEFORE INSERT ON drafts FOR EACH ROW EXECUTE FUNCTION import_failure()").execute(&native).await.unwrap();
    let (dir, source, _, a, _) = fixture().await;
    let draft:Draft=serde_json::from_value(serde_json::json!({"id":"interrupted-import","account_id":a.id(),"revision":0,"to":[],"cc":[],"bcc":[],"subject":"Durable","text":"Body","html":"","in_reply_to":null,"references":[],"attachments":[],"saved_at":""})).unwrap();
    source.save_draft(&a, draft).await.unwrap();
    let archive = dir.path().join("transfer");
    source.backup(&archive).await.unwrap();
    let destination = dir.path().join("incomplete-import");
    assert!(
        Store::import_backend(&archive, &destination, [11; 32], Some(url))
            .await
            .is_err()
    );
    // fixture() uses key 31. A wrong key must not reach the external destination.
    assert!(control.all_accounts().await.unwrap().is_empty());
    let actual_key = [31; 32];
    let destination = dir.path().join("interrupted-import");
    assert!(
        Store::import_backend(&archive, &destination, actual_key, Some(url))
            .await
            .is_err()
    );
    assert!(destination.join("backend-import-in-progress").exists());
    assert_eq!(
        control.setting("backend_import").await.unwrap().as_deref(),
        Some("incomplete")
    );
    assert!(
        Store::open_with_database(&destination, actual_key, 1024, Some(url))
            .await
            .is_err()
    );
    assert!(Store::open_with_database(
        dir.path().join("another-root"),
        actual_key,
        1024,
        Some(url)
    )
    .await
    .is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM drafts")
        .fetch_one(&native)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(source.draft(&a, "interrupted-import").await.is_ok());
    native.close().await;
}
