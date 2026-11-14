use super::*;
use axum::{
    extract::{Query, State},
    Extension,
};
async fn run(
    app: &crate::App,
    user: &str,
    q: &str,
    accounts: &str,
    token: Option<String>,
) -> crate::Result<SearchPage> {
    Ok(crate::search::search(State(app.clone()),Extension(crate::Identity{user:user.into(),csrf:"fixture".into(),token_hash:"fixture".into()}),Query(serde_json::from_value(serde_json::json!({"q":q,"accounts":accounts,"cache_first":true,"continuation":token})).unwrap())).await?.0)
}
#[tokio::test]
async fn indexing_status_waits_for_semantics_and_keeps_local_work_active_through_reconnect() {
    let (_dir, mut app, user, a, _provider) = controlled().await;
    let revision = "status-semantic-fixture";
    app.store.require_search_embeddings(revision).unwrap();
    app.store.configure_search_database(None).await.unwrap();
    app.store.maintain_search_vectors(false).await.unwrap();
    app.store
        .account_state(&a, AccountState::Ready)
        .await
        .unwrap();
    let message:Message=serde_json::from_value(serde_json::json!({"id":"status-message","account_id":a.id(),"thread_id":"status-thread","provider_id":"status-source","folder_ids":["INBOX"],"sender":{"email":"synthetic@example.invalid","name":null},"to":[],"subject":"Status","preview":"","received_at":"2020-01-01T00:00:00.000Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":1})).unwrap();
    let message = app.store.upsert_message(&a, message).await.unwrap();
    app.store.index_search_headers(&a).await.unwrap();
    app.store
        .put_search_body(&a, &message, "", "encrypted")
        .await
        .unwrap();
    app.store.flush_search_index(&a).await.unwrap();
    app.store.put_provider_document(&a,"sync:folders",&serde_json::json!({"folders":[{"name":"INBOX","selectable":true,"attributes":[]}],"index":0,"refreshed":chrono::Utc::now().timestamp()}).to_string()).await.unwrap();
    let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let scan:SearchPage=serde_json::from_value(serde_json::json!({"rows":[message],"continuation":null,"coverage":[{"account_id":a.id(),"folder_ids":["INBOX"],"state":"complete","error":null}],"classification":{"classified":0,"total":1},"pending":false,"counts_exact":false})).unwrap();
    app.store
        .record_scan(&a, "INBOX", generation, &scan)
        .await
        .unwrap();
    let status = || {
        crate::search_status::status(
            State(app.clone()),
            Extension(crate::Identity {
                user: user.clone(),
                csrf: "fixture".into(),
                token_hash: "fixture".into(),
            }),
            axum::extract::Path(a.id().to_owned()),
        )
    };
    let initial = status().await.unwrap().0;
    assert_eq!(initial.state, IndexingState::Paused);
    assert_eq!(
        (initial.processed, initial.eligible, initial.backlog),
        (0, 1, 1)
    );
    app.store
        .prepare_embeddings(&a, revision, true)
        .await
        .unwrap();
    app.store.discover_embeddings(&a, revision).await.unwrap();
    app.store
        .account_state(&a, AccountState::Reconnect)
        .await
        .unwrap();
    assert_eq!(status().await.unwrap().0.state, IndexingState::Indexing);
    let turn = app
        .store
        .claim_embedding(&a, revision, true)
        .await
        .unwrap()
        .unwrap();
    app.store
        .defer_embedding(&a, &turn, "model_failed")
        .await
        .unwrap();
    assert_eq!(status().await.unwrap().0.state, IndexingState::Error);
    // Retry recovery is checked by storage contracts. Expire only this synthetic
    // fixture lease's backoff so the endpoint can exercise final completion.
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite:{}",
        _dir.path().join("search/index.sqlite").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE embedding_jobs SET retry_at=0 WHERE account_id=?")
        .bind(a.id())
        .execute(&db)
        .await
        .unwrap();
    let turn = app
        .store
        .claim_embedding(&a, revision, true)
        .await
        .unwrap()
        .unwrap();
    let mut vector = vec![0.; 384];
    vector[0] = 1.;
    let passage = dakia_storage::EmbeddedPassage {
        text: turn.text.clone(),
        byte_end: turn.text.len(),
        next_offset: turn.text.len(),
        vector: Some(vector),
    };
    app.store
        .commit_embedding(&a, &turn, &passage)
        .await
        .unwrap();
    assert_eq!(status().await.unwrap().0.processed, 0);
    while app.store.advance_search_lexical(&a, true).await.unwrap() {}
    app.store.flush_embedding_receipts(&a).await.unwrap();
    // Saved embeddings are not searchable until the account's vector replay catches up.
    app.store
        .control_repository()
        .put_setting("jobs_enabled", "false")
        .await
        .unwrap();
    let finishing = status().await.unwrap().0;
    assert_eq!(finishing.state, IndexingState::Indexing);
    assert_eq!((finishing.processed, finishing.backlog), (1, 1));
    while app.store.maintain_search_vectors(true).await.unwrap() {}
    app.store
        .control_repository()
        .put_setting("jobs_enabled", "true")
        .await
        .unwrap();
    let complete = status().await.unwrap().0;
    assert_eq!(complete.state, IndexingState::Complete);
    assert_eq!(
        (complete.processed, complete.backlog, complete.gaps),
        (1, 0, 1)
    );
    // Retained completion receipts do not hide a newly unavailable model or
    // failed publication. Terminal content gaps still permit final completion.
    app.store
        .prepare_embeddings(&a, revision, false)
        .await
        .unwrap();
    let unavailable = status().await.unwrap().0;
    assert_eq!(unavailable.state, IndexingState::Error);
    assert_eq!((unavailable.processed, unavailable.backlog), (1, 0));
    app.store
        .prepare_embeddings(&a, revision, true)
        .await
        .unwrap();
    assert_eq!(status().await.unwrap().0.state, IndexingState::Complete);
    app.store
        .put_provider_document(&a, "search:delivery_error", "indexing_failed")
        .await
        .unwrap();
    assert_eq!(status().await.unwrap().0.state, IndexingState::Error);
    app.store
        .put_provider_document(&a, "search:delivery_error", "")
        .await
        .unwrap();
    assert_eq!(status().await.unwrap().0.state, IndexingState::Complete);
    // The old completed generation cannot hide discovery after an epoch change,
    // even when all retained content receipts are already credited.
    app.store
        .put_provider_document(&a, "thread:epoch:INBOX", "2")
        .await
        .unwrap();
    let changed = status().await.unwrap().0;
    assert!(!changed.discovery_complete);
    assert_ne!(changed.state, IndexingState::Complete);
    db.close().await;
}
#[test]
fn full_text_mime_decodes_both_alternatives_unicode_and_declares_gaps() {
    let raw=b"From: f@example.invalid\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nplain caf=C3=A9\r\n--x\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>html-only &amp; other</p>\r\n--x--\r\n";
    let (text, state) = crate::sync::full_text::extract(raw);
    assert_eq!(state, "ready");
    assert!(
        text.contains("café") && text.contains("html-only & other"),
        "{text}"
    );
    assert_eq!(
        crate::sync::full_text::extract(
            b"MIME-Version: 1.0\r\nContent-Type: application/pkcs7-mime\r\n\r\nencrypted"
        )
        .1,
        "encrypted"
    );
    let raw = format!(
        "Content-Type: text/plain\r\n\r\n{}",
        "x".repeat(1024 * 1024 + 1)
    );
    assert_eq!(
        crate::sync::full_text::extract(raw.as_bytes()).1,
        "oversized"
    );
}
#[tokio::test]
async fn full_text_backfill_fetches_read_only_resumes_and_does_not_cache_attachment_payloads() {
    let (dir, app, _user, a, provider) = controlled().await;
    app.mail.probe(&app.store, &a).await.unwrap();
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    *provider.raw.lock().await=b"From: fixture@example.invalid\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>historical-body-only caf\xc3\xa9</p>".to_vec();
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    assert_eq!(app.store.text_coverage(&a).await.unwrap().bodies, 2);
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 2);
    // Completed rows are durable and must not be fetched again on re-entry.
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 2);
    let db = database(&dir, &a).await;
    let cached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM received_cache")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(cached, 0);
    app.store
        .put_provider_document(&a, "search:body_due", "0")
        .await
        .unwrap();
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    assert_eq!(app.store.text_coverage(&a).await.unwrap().bodies, 2);
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 2);
    assert!(provider.mutations.lock().await.is_empty());
}
#[tokio::test]
async fn full_text_federated_pages_are_ordered_complete_is_verified_and_fallback_is_bound() {
    let (_dir, mut app, user, a, provider) = controlled().await;
    app.store.configure_search_database(None).await.unwrap();
    app.store.maintain_search_vectors(false).await.unwrap();
    let input = app.store.credential(&a).await.unwrap();
    let second = app.store.connect(&user, &input).await.unwrap();
    let b = app.store.authorize(&user, &second.id).await.unwrap();
    let mut expected = std::collections::BTreeSet::new();
    for scope in [&a, &b] {
        let mut rows = Vec::new();
        for i in 0..123 {
            let id = format!("history-{i:03}");
            let m:Message=serde_json::from_value(serde_json::json!({"id":id,"account_id":scope.id(),"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"synthetic@example.invalid","name":null},"to":[],"subject":"Old mail","preview":"","received_at":"2020-01-01T00:00:00.000Z","unread":false,"starred":false,"category":null,"category_status":"pending","revision":1})).unwrap();
            rows.push(app.store.upsert_message(scope, m).await.unwrap());
            expected.insert(format!("{}:{id}", scope.id()));
        }
        app.store.index_search_headers(scope).await.unwrap();
        for m in &rows {
            app.store
                .put_search_body(scope, m, "body-only 東京 café", "ready")
                .await
                .unwrap();
        }
        app.store.flush_search_index(scope).await.unwrap();
        for _ in 0..10 {
            if !app.store.advance_search_lexical(scope, true).await.unwrap() {
                break;
            }
        }
        app.store.put_provider_document(scope,"sync:folders",&serde_json::json!({"folders":[{"name":"INBOX","selectable":true,"attributes":[]}],"index":0,"refreshed":chrono::Utc::now().timestamp()}).to_string()).await.unwrap();
        let (generation, _) = app.store.scan_state(scope, "INBOX").await.unwrap().unwrap();
        let scan = SearchPage {
            rows,
            continuation: None,
            coverage: vec![AccountCoverage {
                account_id: scope.id().into(),
                folder_ids: vec!["INBOX".into()],
                state: CoverageState::Complete,
                error: None,
                inbox: None,
            }],
            classification: ClassificationCoverage {
                classified: 0,
                total: 123,
            },
            pending: false,
            counts_exact: false,
            cache_preview: None,
            index_page: None,
            search: None,
            supplement: None,
        };
        app.store
            .record_scan(scope, "INBOX", generation, &scan)
            .await
            .unwrap();
    }
    let accounts = format!("{},{}", a.id(), b.id());
    let mut token = None;
    let mut got = std::collections::BTreeSet::new();
    let mut order = Vec::new();
    let start = std::time::Instant::now();
    loop {
        let page = run(&app, &user, "cafe\u{301}", &accounts, token)
            .await
            .unwrap();
        assert!(!page.pending && page.cache_preview.is_none());
        assert!(page
            .coverage
            .iter()
            .all(|c| c.state == CoverageState::Complete));
        assert!(page.rows.len() <= 50);
        for m in page.rows {
            let key = format!("{}:{}", m.account_id, m.id);
            assert!(got.insert(key));
            order.push((m.account_id, m.id));
        }
        token = page.continuation;
        if token.is_none() {
            break;
        }
    }
    assert_eq!(got, expected);
    let mut sorted = order.clone();
    sorted.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    assert_eq!(order, sorted);
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), 0);
    println!(
        "full-text handler federated=246 exhausted_ms={:.2}",
        start.elapsed().as_secs_f64() * 1000.
    );
    // A gap or stale scan changes truth immediately, not just after a restart.
    app.store.reset_scan(&a, "INBOX").await.unwrap();
    let first = run(&app, &user, "café", a.id(), None).await.unwrap();
    assert!(first.pending && first.cache_preview == Some(true));
    assert_eq!(
        run(&app, &user, "different", a.id(), first.continuation.clone())
            .await
            .unwrap_err()
            .0,
        axum::http::StatusCode::BAD_REQUEST
    );
    let other = app.store.user("fixture", "not-owner").await.unwrap();
    assert!(
        run(&app, &other, "café", a.id(), first.continuation.clone())
            .await
            .is_err()
    );
    let mut token = first.continuation;
    let mut provider_seen = false;
    for _ in 0..10 {
        let page = run(&app, &user, "café", a.id(), token).await.unwrap();
        provider_seen |= page.cache_preview.is_none();
        token = page.continuation;
        if token.is_none() {
            break;
        }
    }
    assert!(provider_seen && provider.base.scans.load(Ordering::SeqCst) > 0);
    assert!(provider.mutations.lock().await.is_empty());
}

#[tokio::test]
async fn permanent_index_failure_is_not_fetched_again_when_old_retry_time_expires() {
    let (dir, app, _user, a, provider) = controlled().await;
    app.mail.probe(&app.store, &a).await.unwrap();
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    *provider.raw.lock().await =
        b"MIME-Version: 1.0\r\nContent-Type: application/pkcs7-mime\r\n\r\nencrypted".to_vec();
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 2);
    let db = database(&dir, &a).await;
    sqlx::query("UPDATE search_documents SET body_state='ready' WHERE body_state='pending'")
        .execute(&db)
        .await
        .unwrap();
    for _ in 0..48 {
        sqlx::query("UPDATE search_documents SET retry_at=0")
            .execute(&db)
            .await
            .unwrap();
        app.store
            .put_provider_document(&a, "search:body_due", "0")
            .await
            .unwrap();
        crate::sync::full_text::tick(&app, &a).await.unwrap();
    }
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 2);
    assert!(provider.mutations.lock().await.is_empty());
}

#[tokio::test]
async fn stale_imap_body_read_revokes_snapshot_and_reconciles_before_pruning() {
    let (dir, app, _user, a, provider) = controlled().await;
    {
        let mut page = provider.page.lock().await;
        page.full_coverage = true;
        for row in &mut page.rows {
            row.identity = p::Identity::Imap(row.reference.clone());
        }
    }
    app.mail.probe(&app.store, &a).await.unwrap();
    let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    app.mail
        .record_snapshot_page(&app.store, &a, "INBOX", generation, None)
        .await
        .unwrap();
    app.store
        .confirm_unchanged_scan(&a, "INBOX", generation)
        .await
        .unwrap();
    assert!(app
        .store
        .verified_snapshot_current(&a, "INBOX")
        .await
        .unwrap());
    let before = app.store.inbox_rows(&a, None, None).await.unwrap();
    assert_eq!(before.len(), 2);
    *provider.failure.lock().await = Some(p::ErrorKind::StaleReference);
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    assert!(!app
        .store
        .verified_snapshot_current(&a, "INBOX")
        .await
        .unwrap());
    assert_eq!(
        app.store.scan_state(&a, "INBOX").await.unwrap(),
        Some((generation + 1, None))
    );
    assert_eq!(
        app.store.inbox_rows(&a, None, None).await.unwrap().len(),
        2,
        "failed read alone must never delete membership"
    );
    // Further failures must not reset an in-flight traversal or replay writes.
    app.store
        .put_provider_document(&a, "search:body_due", "0")
        .await
        .unwrap();
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    assert_eq!(
        app.store.scan_state(&a, "INBOX").await.unwrap(),
        Some((generation + 1, None))
    );
    *provider.failure.lock().await = None;
    provider.page.lock().await.rows.clear();
    app.mail
        .record_snapshot_page(&app.store, &a, "INBOX", generation + 1, None)
        .await
        .unwrap();
    assert!(app
        .store
        .inbox_rows(&a, None, None)
        .await
        .unwrap()
        .is_empty());
    app.store.index_search_headers(&a).await.unwrap();
    assert!(app
        .store
        .next_search_body(&a, false)
        .await
        .unwrap()
        .is_none());
    assert!(provider.mutations.lock().await.is_empty());
    let db = database(&dir, &a).await;
    let errors: i64 =
        sqlx::query_scalar("SELECT count(*) FROM search_documents WHERE body_state='fetch_error'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(errors, 0);
}

#[tokio::test]
async fn untracked_imap_membership_cannot_reuse_an_older_completed_snapshot() {
    let (dir, app, _user, a, provider) = controlled().await;
    {
        let mut page = provider.page.lock().await;
        page.full_coverage = true;
        for row in &mut page.rows {
            row.identity = p::Identity::Imap(row.reference.clone());
        }
    }
    app.mail.probe(&app.store, &a).await.unwrap();
    let (generation, _) = app.store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    app.mail
        .record_snapshot_page(&app.store, &a, "INBOX", generation, None)
        .await
        .unwrap();
    let old = app.store.inbox_rows(&a, None, None).await.unwrap();
    let db = database(&dir, &a).await;
    // Reproduce the production mismatch without changing the visible record.
    sqlx::query("DELETE FROM folder_memberships WHERE message_id=?")
        .bind(&old[0].id)
        .execute(&db)
        .await
        .unwrap();
    app.store
        .confirm_unchanged_scan(&a, "INBOX", generation)
        .await
        .unwrap();
    sqlx::query("UPDATE folder_scans SET last_scanned_at=0 WHERE folder='INBOX'")
        .execute(&db)
        .await
        .unwrap();
    let (scan, verified) = app
        .mail
        .prepare_scan(&app.store, &a, "INBOX")
        .await
        .unwrap();
    assert!(!verified);
    assert_eq!(scan, Some((generation + 1, None)));
    assert!(!app
        .store
        .verified_snapshot_current(&a, "INBOX")
        .await
        .unwrap());
    assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 2);
    provider
        .page
        .lock()
        .await
        .rows
        .retain(|m| serde_json::to_string(&m.reference).unwrap() != old[0].provider_id);
    app.mail
        .record_snapshot_page(&app.store, &a, "INBOX", generation + 1, None)
        .await
        .unwrap();
    assert!(app
        .store
        .message(&a, &old[0].id)
        .await
        .unwrap()
        .folder_ids
        .is_empty());
    assert_eq!(app.store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
    assert!(provider.mutations.lock().await.is_empty());
}

async fn gmail_reference_fixture() -> (
    tempfile::TempDir,
    crate::App,
    String,
    AuthorizedAccount,
    Arc<Controlled>,
    Message,
) {
    let (dir, app, user, a, provider) = controlled().await;
    {
        let mut page = provider.page.lock().await;
        page.rows.truncate(1);
        let r = &mut page.rows[0];
        r.identity = p::Identity::Gmail {
            account_id: a.id().into(),
            message_id: 99,
        };
    }
    app.mail.probe(&app.store, &a).await.unwrap();
    let rows = app
        .mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap()
        .rows;
    (dir, app, user, a, provider, rows[0].clone())
}
async fn index_turn(app: &crate::App, a: &AuthorizedAccount) {
    app.store
        .put_provider_document(a, "search:body_due", "0")
        .await
        .unwrap();
    crate::sync::full_text::tick(app, a).await.unwrap();
}
#[tokio::test]
async fn gmail_unavailable_references_stop_retrying_and_fresh_observations_restore_indexing() {
    let (dir, app, _user, a, provider, m) = gmail_reference_fixture().await;
    *provider.failure.lock().await = Some(p::ErrorKind::StaleReference);
    index_turn(&app, &a).await;
    let db = database(&dir, &a).await;
    let state: String =
        sqlx::query_scalar("SELECT body_state FROM search_documents WHERE message_id=?")
            .bind(&m.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(state, "reference_unavailable");
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 1);
    // Hours and folder generations cannot manufacture positive UID evidence.
    for _ in 0..48 {
        sqlx::query("UPDATE search_documents SET retry_at=0")
            .execute(&db)
            .await
            .unwrap();
        app.store.reset_scan(&a, "INBOX").await.unwrap();
        index_turn(&app, &a).await;
        assert!(app.mail.raw(&app.store, &a, &m.id).await.is_err());
    }
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.recovery_calls.load(Ordering::SeqCst),
        1,
        "durable account lookup budget survives repeated scheduler turns"
    );
    assert!(!app
        .store
        .message(&a, &m.id)
        .await
        .unwrap()
        .folder_ids
        .is_empty());
    *provider.failure.lock().await = None;
    {
        let mut page = provider.page.lock().await;
        page.rows[0].reference.uid += 10;
    }
    let refreshed = app
        .mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    assert_eq!(
        refreshed.rows[0].id, m.id,
        "Gmail identity is stable across UID replacement"
    );
    index_turn(&app, &a).await;
    assert_eq!(app.store.text_coverage(&a).await.unwrap().bodies, 1);
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 2);
    assert!(provider.mutations.lock().await.is_empty());
}
#[tokio::test]
async fn gmail_same_uid_positive_reobservation_restores_indexing_and_old_failure_cannot_override_it(
) {
    let (_dir, app, _user, a, provider, m) = gmail_reference_fixture().await;
    app.store.index_search_headers(&a).await.unwrap();
    let old = app.store.content_references(&a, &m).await.unwrap();
    let reference = app
        .store
        .provider_document(&a, &format!("reference:{}", m.id))
        .await
        .unwrap()
        .unwrap();
    *provider.failure.lock().await = Some(p::ErrorKind::StaleReference);
    index_turn(&app, &a).await;
    *provider.failure.lock().await = None;
    // Cached UI metadata is not fresh provider evidence for the same UID.
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    assert!(app
        .store
        .next_search_body(&a, false)
        .await
        .unwrap()
        .is_none());
    app.mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap();
    assert!(!app
        .store
        .block_content_reference(&a, &m, &reference, &old.token)
        .await
        .unwrap());
    assert!(!app
        .store
        .mark_reference_gap(&a, &m, &old.token)
        .await
        .unwrap());
    index_turn(&app, &a).await;
    assert_eq!(app.store.text_coverage(&a).await.unwrap().bodies, 1);
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn gmail_bounded_identity_discovery_rearms_same_message_without_unpruning_the_mailbox() {
    let (_dir, app, _user, a, provider, m) = gmail_reference_fixture().await;
    *provider.failure.lock().await = Some(p::ErrorKind::StaleReference);
    index_turn(&app, &a).await;
    let mut found = provider.page.lock().await.rows[0].clone();
    found.reference.uid += 50;
    *provider.recovered.lock().await = Some(found);
    *provider.failure.lock().await = None;
    assert!(app
        .mail
        .recover_content_reference(&app.store, &a)
        .await
        .unwrap());
    assert!(!app
        .mail
        .recover_content_reference(&app.store, &a)
        .await
        .unwrap());
    index_turn(&app, &a).await;
    assert_eq!(
        app.store.message(&a, &m.id).await.unwrap().provider_id,
        "gmail:99"
    );
    assert_eq!(app.store.text_coverage(&a).await.unwrap().bodies, 1);
    assert_eq!(provider.recovery_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn anydoc_attachment_text_and_all_available_participants_are_extracted_locally() {
    let raw = b"From: Sender <sender@example.invalid>\r\nTo: recipient@example.invalid\r\nCc: Carbon Person <carbon@example.invalid>\r\nBcc: hidden@example.invalid\r\nReply-To: replies@example.invalid\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=boundary\r\n\r\n--boundary\r\nContent-Type: text/plain\r\n\r\nordinary message\r\n--boundary\r\nContent-Type: application/rtf\r\nContent-Disposition: attachment; filename=report.rtf\r\n\r\n{\\rtf1\\ansi confidential funding figures}\r\n--boundary\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=unknown.bin\r\n\r\nnot a document\r\n--boundary--\r\n";
    let extracted = crate::sync::full_text::extract_content(raw);
    assert_eq!(extracted.state, "partial");
    assert!(extracted.text.contains("ordinary message"));
    assert!(
        extracted
            .attachments
            .contains("confidential funding figures"),
        "{}",
        extracted.attachments
    );
    for value in [
        "sender@example.invalid",
        "recipient@example.invalid",
        "carbon@example.invalid",
        "hidden@example.invalid",
        "replies@example.invalid",
    ] {
        assert!(extracted.participants.contains(value), "{value}");
    }
    assert!(extracted.recipients.contains("carbon@example.invalid"));
    let details: serde_json::Value = serde_json::from_str(&extracted.details).unwrap();
    assert_eq!(details[0]["state"], "ready");
    assert_eq!(details[1]["state"], "unsupported");
}

#[test]
fn content_gaps_preserve_readable_headers_and_attachment_names() {
    let encrypted = b"From: Sender <sender@example.invalid>\r\nCc: Carbon <carbon@example.invalid>\r\nReply-To: replies@example.invalid\r\nContent-Type: multipart/encrypted; boundary=e\r\n\r\n--e\r\nContent-Type: application/pgp-encrypted\r\nContent-Disposition: attachment; filename=secret.pgp\r\n\r\nciphertext must not be indexed\r\n--e--\r\n";
    let content = crate::sync::full_text::extract_content(encrypted);
    assert_eq!(content.state, "encrypted");
    assert!(content.text.is_empty());
    assert!(content.participants.contains("replies@example.invalid"));
    assert!(content.recipients.contains("carbon@example.invalid"));
    assert!(content.attachments.contains("secret.pgp"));
    assert!(!content.attachments.contains("ciphertext"));
    let details: serde_json::Value = serde_json::from_str(&content.details).unwrap();
    assert_eq!(details[0]["state"], "encrypted");

    let raw = format!("From: sender@example.invalid\r\nBcc: hidden@example.invalid\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=retained.txt\r\n\r\nattachment survives a large body\r\n--b--\r\n", "ü".repeat(600_000));
    let content = crate::sync::full_text::extract_content(raw.as_bytes());
    assert_eq!(content.state, "oversized");
    assert!(content.text.len() <= 1024 * 1024);
    assert!(content.participants.contains("sender@example.invalid"));
    assert!(content.recipients.contains("hidden@example.invalid"));
    assert!(content
        .attachments
        .contains("attachment survives a large body"));
}

#[test]
fn an_encrypted_part_does_not_hide_readable_sibling_content() {
    let raw = b"From: sender@example.invalid\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nreadable sibling message\r\n--b\r\nContent-Type: multipart/encrypted; boundary=e\r\n\r\n--e\r\nContent-Type: text/plain\r\n\r\nsecret ciphertext marker\r\n--e--\r\n--b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=readable.txt\r\n\r\nreadable sibling attachment\r\n--b--\r\n";
    let content = crate::sync::full_text::extract_content(raw);
    assert_eq!(content.state, "partial");
    assert!(content.text.contains("readable sibling message"));
    assert!(content.attachments.contains("readable sibling attachment"));
    assert!(!content.text.contains("secret ciphertext marker"));
    assert!(!content.attachments.contains("secret ciphertext marker"));
}

#[test]
fn attachment_spans_use_normalized_utf8_byte_offsets() {
    let raw = "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=first.txt\r\n\r\nİSTANBUL e\u{301}\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=second.txt\r\n\r\nSECOND VALUE\r\n--b--\r\n";
    let content = crate::sync::full_text::extract_content(raw.as_bytes());
    assert_eq!(content.state, "ready");
    assert_eq!(
        content.attachments,
        dakia_storage::normalize_search(&content.attachments)
    );
    let details: serde_json::Value = serde_json::from_str(&content.details).unwrap();
    let mut offset = 0;
    for part in details.as_array().unwrap() {
        let start = part["byte_start"].as_u64().unwrap() as usize;
        let end = part["byte_end"].as_u64().unwrap() as usize;
        assert_eq!(start, offset);
        let text = &content.attachments[start..end];
        assert!(text.starts_with(part["filename"].as_str().unwrap()));
        assert!(text.contains(if start == 0 {
            "i\u{307}stanbul é"
        } else {
            "second value"
        }));
        offset = end;
    }
    assert_eq!(offset, content.attachments.len());
}

#[test]
fn quoted_index_terms_preserve_phrases_and_exclude_only_known_filters() {
    assert_eq!(
        crate::search::index_units(
            r#"from:"Maya Person" "quarterly report" budget is:unread https://example.invalid"#
        ),
        vec!["quarterly report", "budget", "https://example.invalid"]
    );
}

#[tokio::test]
async fn broken_independent_index_keeps_provider_search_available() {
    let (dir, mut app, user, a, provider) = controlled().await;
    app.store.configure_search_database(None).await.unwrap();
    app.store.maintain_search_vectors(false).await.unwrap();
    app.mail.probe(&app.store, &a).await.unwrap();
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    *provider.raw.lock().await = b"Content-Type: text/plain\r\n\r\nunique body token".to_vec();
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    let index = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("search/index.sqlite")),
    )
    .await
    .unwrap();
    sqlx::query("DROP TABLE lexical_terms")
        .execute(&index)
        .await
        .unwrap();
    let before = provider.base.scans.load(Ordering::SeqCst);
    let found = run(&app, &user, "unique body token", a.id(), None)
        .await
        .unwrap();
    assert_eq!(
        provider.base.scans.load(Ordering::SeqCst),
        before,
        "index failure must return without waiting for provider I/O"
    );
    let supplement = found.supplement.unwrap().remove(0);
    let found = run(
        &app,
        &user,
        "unique body token",
        a.id(),
        Some(supplement.continuation),
    )
    .await
    .unwrap();
    assert!(provider.base.scans.load(Ordering::SeqCst) > before);
    assert!(!found.rows.is_empty());
    assert!(found
        .search
        .unwrap()
        .degradation
        .iter()
        .any(|d| d.code == "provider_literal_only"));
    assert!(provider.mutations.lock().await.is_empty());
}

#[tokio::test]
async fn independent_sqlite_index_returns_body_results_without_waiting_for_provider() {
    let (_dir, mut app, user, a, provider) = controlled().await;
    app.store.configure_search_database(None).await.unwrap();
    app.store.maintain_search_vectors(false).await.unwrap();
    app.mail.probe(&app.store, &a).await.unwrap();
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    *provider.raw.lock().await =
        b"Content-Type: text/plain\r\n\r\nuniquebody project report".to_vec();
    crate::sync::full_text::tick(&app, &a).await.unwrap();
    let before = provider.base.scans.load(Ordering::SeqCst);
    let found = run(&app, &user, "uniquebody report", a.id(), None)
        .await
        .unwrap();
    assert_eq!(found.rows.len(), 2);
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), before);
    assert_eq!(
        app.store.text_coverage(&a).await.unwrap().delivery_pending,
        0
    );
}

#[tokio::test]
async fn unavailable_search_store_pauses_acquisition_reports_status_and_recovers() {
    let (dir, mut app, user, a, provider) = controlled().await;
    let directory = dir.path().join("unavailable-search");
    let url = format!("sqlite:{}", directory.join("index.sqlite").display());
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        app.store.configure_search_database(Some(&url)),
    )
    .await
    .expect("search startup must not block mail")
    .unwrap();
    app.mail.probe(&app.store, &a).await.unwrap();
    app.mail
        .page(&app.store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap();
    let fetched = provider.fetch_calls.load(Ordering::SeqCst);
    assert!(!crate::sync::full_text::tick(&app, &a).await.unwrap());
    assert_eq!(
        provider.fetch_calls.load(Ordering::SeqCst),
        fetched,
        "do not keep acquiring bodies into staging during a search outage"
    );
    let status = crate::search_status::status(
        State(app.clone()),
        Extension(crate::Identity {
            user: user.clone(),
            csrf: "fixture".into(),
            token_hash: "fixture".into(),
        }),
        axum::extract::Path(a.id().to_owned()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(status.state, IndexingState::Error);
    assert_eq!(status.processed, 0);
    assert!(status.backlog > 0);
    let before = provider.base.scans.load(Ordering::SeqCst);
    let first = run(&app, &user, "provider fallback", a.id(), None)
        .await
        .unwrap();
    let token = first.supplement.unwrap().remove(0).continuation;
    assert!(!run(&app, &user, "provider fallback", a.id(), Some(token))
        .await
        .unwrap()
        .rows
        .is_empty());
    assert!(provider.base.scans.load(Ordering::SeqCst) > before);
    tokio::fs::create_dir_all(&directory).await.unwrap();
    *provider.raw.lock().await = b"Content-Type: text/plain\r\n\r\nrecoveredbody token".to_vec();
    let recovery_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        crate::sync::full_text::tick(&app, &a).await.unwrap();
        if provider.fetch_calls.load(Ordering::SeqCst) > fetched {
            break;
        }
        assert!(
            tokio::time::Instant::now() < recovery_deadline,
            "search acquisition did not resume after reconnection"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let before = provider.base.scans.load(Ordering::SeqCst);
    assert_eq!(
        run(&app, &user, "recoveredbody", a.id(), None)
            .await
            .unwrap()
            .rows
            .len(),
        2
    );
    assert_eq!(provider.base.scans.load(Ordering::SeqCst), before);
    assert!(provider.mutations.lock().await.is_empty());
}

#[path = "hybrid_search_tests.rs"]
mod hybrid_search_tests;

#[path = "search_scale_tests.rs"]
mod search_scale_tests;
