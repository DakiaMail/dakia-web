use super::*;
use axum::{
    extract::{Query, State},
    Extension,
};

async fn run(
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
async fn search_preview_avoids_slow_provider_then_reconciles_and_binds_cursor() {
    let (_dir, app, user, a, provider) = controlled().await;
    app.mail.probe(&app.store, &a).await.unwrap();
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    *provider.search_delay.lock().await = Duration::from_millis(250);
    let request =
        serde_json::json!({"q":"subject:Synthetic", "accounts":a.id(), "cache_first":true});
    let scans = provider.base.scans.load(Ordering::SeqCst);
    let start = std::time::Instant::now();
    let preview = run(&app, &user, request.clone()).await.unwrap();
    let preview_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), scans);
    assert_eq!(preview.cache_preview, Some(true));
    assert_eq!(preview.rows.len(), 2);
    assert!(preview.pending && !preview.counts_exact);
    assert!(preview
        .coverage
        .iter()
        .all(|c| c.state == CoverageState::Pending));
    let mut next = request.clone();
    next["continuation"] = preview.continuation.clone().unwrap().into();
    let start = std::time::Instant::now();
    let live = run(&app, &user, next.clone()).await.unwrap();
    let provider_ms = start.elapsed().as_secs_f64() * 1000.;
    assert!(
        preview_ms < provider_ms,
        "local {preview_ms} ms, provider {provider_ms} ms"
    );
    assert_eq!(live.rows.len(), 2); // preview IDs do not suppress refreshed rows
    assert_eq!(live.cache_preview, None);
    let replay = run(&app, &user, next.clone()).await.unwrap();
    assert_eq!(live, replay);
    next["q"] = "subject:other".into();
    assert_eq!(
        run(&app, &user, next).await.unwrap_err().0,
        axum::http::StatusCode::BAD_REQUEST
    );
    let other = app.store.user("fixture", "other-preview").await.unwrap();
    assert_eq!(
        run(&app, &other, request.clone()).await.unwrap_err().0,
        axum::http::StatusCode::NOT_FOUND
    );
    *provider.failure.lock().await = Some(p::ErrorKind::Retryable);
    let saved = run(&app, &user, request.clone()).await.unwrap();
    assert_eq!(saved.rows.len(), 2);
    let mut next = request;
    next["continuation"] = saved.continuation.unwrap().into();
    let failed = run(&app, &user, next).await.unwrap();
    assert!(failed
        .coverage
        .iter()
        .any(|c| c.state == CoverageState::Error));
    assert!(!failed.counts_exact);
    assert!(provider.mutations.lock().await.is_empty());
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 0);
    println!("search preview: local={preview_ms:.2}ms provider={provider_ms:.2}ms artificial_provider_delay=250ms");
}

#[tokio::test]
async fn search_preview_preserves_filters_and_never_guesses_body_dates_or_folder_roles() {
    let (_dir, app, user, a, provider) = controlled().await;
    app.mail.probe(&app.store, &a).await.unwrap();
    let rows = app
        .mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap()
        .rows;
    let mut row = rows[0].clone();
    row.subject = "Invoice 100%_ literal".into();
    row.sender = Address {
        email: "list+tag@example.invalid".into(),
        name: Some("Sender Name".into()),
    };
    row.to = vec![Address {
        email: "receiver@example.invalid".into(),
        name: Some("Receiver".into()),
    }];
    row.starred = true;
    row.unread = true;
    row.category = Some("people".into());
    app.store.upsert_message(&a, row.clone()).await.unwrap();
    let ask = |q: &str| serde_json::json!({"q":q,"accounts":a.id(),"cache_first":true});
    for q in [
        "subject:100%_ from:list+tag to:Receiver is:unread is:starred",
        "folder:INBOX subject:Invoice",
        "\"Invoice 100%_\"",
    ] {
        let page = run(&app, &user, ask(q)).await.unwrap();
        assert_eq!(page.cache_preview, Some(true), "{q}");
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].id, row.id);
    }
    for q in [
        "subject:not-present",
        "subject:Invoice is:read",
        "body-only-not-in-headers",
        "subject:Invoice since:2026-01-01",
        "folder:unknown subject:Invoice",
    ] {
        assert_ne!(
            run(&app, &user, ask(q)).await.unwrap().cache_preview,
            Some(true),
            "{q}"
        );
    }
    let mut category = ask("subject:Invoice");
    category["category"] = "newsletters".into();
    assert_ne!(
        run(&app, &user, category).await.unwrap().cache_preview,
        Some(true)
    );
    app.store
        .put_provider_document(&a, "thread:epoch:INBOX", "2")
        .await
        .unwrap();
    assert!(app
        .store
        .cached_search_rows(&a, &Default::default())
        .await
        .unwrap()
        .is_empty());
    assert!(provider.mutations.lock().await.is_empty());
}

#[tokio::test]
async fn search_preview_is_indexed_bounded_and_account_isolated_at_scale() {
    let (dir, app, user, a, _) = controlled().await;
    let db = database(&dir, &a).await;
    let mut tx = db.begin().await.unwrap();
    for i in 0..50_000 {
        let row = serde_json::json!({"id":format!("m{i:06}"),"account_id":a.id(),"thread_id":"synthetic","provider_id":"stable","folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},"to":[],"subject":"Synthetic scale","preview":"","received_at":"2026-10-31T00:00:00.000Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":1});
        sqlx::query("INSERT INTO messages(id,revision,document) VALUES(?,1,?)")
            .bind(format!("m{i:06}"))
            .bind(row.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    let plan: Vec<(i64,i64,i64,String)> = sqlx::query_as("EXPLAIN QUERY PLAN SELECT document FROM messages ORDER BY json_extract(document,'$.received_at') DESC,id DESC LIMIT 2000").fetch_all(&db).await.unwrap();
    assert!(plan.iter().any(|p| p.3.contains("messages_received_order")));
    let start = std::time::Instant::now();
    let candidates = app
        .store
        .cached_search_rows(&a, &Default::default())
        .await
        .unwrap();
    let candidate_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(candidates.len(), 50);
    assert_eq!(candidates[0].id, "m049999");
    let start = std::time::Instant::now();
    let preview = run(
        &app,
        &user,
        serde_json::json!({"q":"subject:Synthetic","accounts":a.id(),"cache_first":true}),
    )
    .await
    .unwrap();
    let preview_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(preview.rows.len(), 50);
    assert_eq!(preview.cache_preview, Some(true));
    let input = app.store.credential(&a).await.unwrap();
    let second = app.store.connect(&user, &input).await.unwrap();
    let b = app.store.authorize(&user, &second.id).await.unwrap();
    assert!(app
        .store
        .cached_search_rows(&b, &Default::default())
        .await
        .unwrap()
        .is_empty());
    // A rare match at the oldest edge must not be omitted by a recent-window cap.
    sqlx::query("UPDATE messages SET document=json_set(document,'$.subject','Rare historical match') WHERE id='m000000'").execute(&db).await.unwrap();
    let start = std::time::Instant::now();
    let historical = run(
        &app,
        &user,
        serde_json::json!({"q":"subject:historical","accounts":a.id(),"cache_first":true}),
    )
    .await
    .unwrap();
    let historical_ms = start.elapsed().as_secs_f64() * 1000.;
    assert_eq!(historical.cache_preview, Some(true));
    assert_eq!(historical.rows[0].id, "m000000");
    println!("search preview 50000 synthetic headers: candidates={} candidate_ms={candidate_ms:.2} preview_rows={} preview_ms={preview_ms:.2} historical_ms={historical_ms:.2}", candidates.len(), preview.rows.len());
}
