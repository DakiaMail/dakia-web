use super::*;
use axum::{
    extract::{Query, State},
    Extension,
};

async fn search_page(
    app: &crate::App,
    user: &str,
    request: serde_json::Value,
) -> crate::Result<SearchPage> {
    Ok(crate::search::search(
        State(app.clone()),
        Extension(crate::Identity {
            user: user.into(),
            csrf: "fixture".into(),
            token_hash: "fixture".into(),
        }),
        Query(serde_json::from_value(request).unwrap()),
    )
    .await?
    .0)
}

#[tokio::test]
async fn federated_category_pagination_deduplicates_memberships_and_replays_cursors() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [6; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"gmail","imap_host":"imap.gmail.com","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.gmail.com","smtp_port":465,"smtp_tls":"implicit","username":"owner@example.invalid","password":"fixture-only"})).unwrap();
    let app = crate::App {
        store,
        config: Arc::new(crate::Config {
            origin: "https://dakia.example.invalid".into(),
            issuer: "https://id.example.invalid".into(),
            client_id: "fixture".into(),
            client_secret: "fixture-only".into(),
            web_root: dir.path().to_string_lossy().into(),
        }),
        mail: MailRuntime::default(),
    };
    let mut ids = vec![];
    for index in 0..3 {
        let account = app.store.connect(&user, &input).await.unwrap();
        app.mail.providers.lock().await.insert(
            account.id.clone(),
            Arc::new(FixtureProvider {
                account: account.id.clone(),
                fetches: AtomicUsize::new(0),
                fetch_error: None,
                probe_error: (index == 2).then(|| {
                    p::error(
                        p::ErrorKind::Authentication,
                        "IMAP probe: fixture rejection",
                    )
                }),
            }),
        );
        ids.push(account.id);
    }
    let request = serde_json::json!({"q":"subject:Synthetic", "accounts": ids.join(","), "category":"people"});
    let first = search_page(&app, &user, request.clone()).await.unwrap();
    assert_eq!(first.rows.len(), 2); // Unknown categories remain visible.
    assert!(first.pending);
    assert_eq!(
        first
            .coverage
            .iter()
            .filter(|c| c.state == CoverageState::Error)
            .count(),
        1
    );
    assert_eq!(first.classification.classified, 0);
    let token = first.continuation.unwrap();
    assert_eq!(token.len(), 36);
    let mut continuation = request.clone();
    continuation["continuation"] = token.into();
    let second = search_page(&app, &user, continuation.clone())
        .await
        .unwrap();
    assert!(second.rows.is_empty()); // Same Gmail IDs in Sent are not duplicated.
    assert!(!second.pending);
    assert!(second.continuation.is_none());
    assert_eq!(
        second
            .coverage
            .iter()
            .filter(|c| c.state == CoverageState::Partial)
            .count(),
        2
    );
    let replay = search_page(&app, &user, continuation.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&second).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    let mut changed = continuation.clone();
    changed["category"] = "newsletters".into();
    assert_eq!(
        search_page(&app, &user, changed).await.unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    let other = app.store.user("fixture", "other").await.unwrap();
    assert_eq!(
        search_page(&app, &other, continuation).await.unwrap_err().0,
        StatusCode::NOT_FOUND
    );

    // An empty filtered first page must still continue provider scanning.
    for row in &first.rows {
        let scope = app.store.authorize(&user, &row.account_id).await.unwrap();
        let mut message = app.store.message(&scope, &row.id).await.unwrap();
        message.category = Some("newsletters".into());
        message.category_status = CategoryStatus::Ready;
        app.store.upsert_message(&scope, message).await.unwrap();
    }
    let filtered = search_page(&app, &user, request.clone()).await.unwrap();
    assert!(filtered.rows.is_empty());
    assert!(filtered.continuation.is_some());
    let mut next = request;
    next["continuation"] = filtered.continuation.unwrap().into();
    let end = search_page(&app, &user, next).await.unwrap();
    assert!(end.rows.is_empty());
    assert!(end.continuation.is_none());

    // The cleanup scope is bound into continuation tokens and excludes the
    // account's explicit Trash mapping, not a guessed English mailbox name.
    let scoped = serde_json::json!({"q":"from:sender@example.invalid", "accounts":ids[0], "exclude_trash":true});
    let first = search_page(&app, &user, scoped.clone()).await.unwrap();
    let mut changed = scoped.clone();
    changed["continuation"] = first.continuation.unwrap().into();
    changed["exclude_trash"] = false.into();
    assert_eq!(
        search_page(&app, &user, changed).await.unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    let a = app.store.authorize(&user, &ids[0]).await.unwrap();
    let mut settings = app.mail.settings(&app.store, &a).await.unwrap();
    settings.trash_folder = Some("Sent Items".into());
    app.store
        .put_provider_document(
            &a,
            "mailbox-settings",
            &serde_json::to_string(&settings).unwrap(),
        )
        .await
        .unwrap();
    let excluded = search_page(&app, &user, scoped).await.unwrap();
    assert_eq!(excluded.coverage[0].folder_ids, vec!["INBOX"]);
    assert!(excluded.continuation.is_none());
}
