use dakia_domain::*;
use dakia_storage::{CleanupRequest, CleanupTarget, Error, Store};

async fn fixture() -> (tempfile::TempDir, Store, String, String, Vec<Message>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [42; 32]).await.unwrap();
    let user = store.user("issuer", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let mut rows = Vec::new();
    for n in 0..43 {
        let m: Message = serde_json::from_value(serde_json::json!({"id":format!("m{n:03}"),"account_id":a.id(),"thread_id":"fixture","provider_id":format!("fixture-{n}"),"folder_ids":["INBOX"],"sender":{"email":"list@example.invalid","name":null},"to":[],"subject":"Synthetic list","preview":"Fixture","received_at":"2026-01-01T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
        rows.push(store.upsert_message(&a, m).await.unwrap());
    }
    (dir, store, user, account.id, rows)
}
fn batch(rows: &[Message]) -> CleanupRequest {
    CleanupRequest {
        id: uuid::Uuid::new_v4().to_string(),
        source_id: rows[0].id.clone(),
        search_complete: false,
        targets: rows
            .iter()
            .map(|m| CleanupTarget {
                id: m.id.clone(),
                expected_revision: m.revision,
                expected_location: None,
            })
            .collect(),
    }
}
#[tokio::test]
async fn pending_cleanup_projects_trash_without_claiming_provider_completion() {
    let (_dir, store, user, account, rows) = fixture().await;
    let a = store.authorize(&user, &account).await.unwrap();
    store
        .enqueue_sender_cleanup(&a, batch(&rows))
        .await
        .unwrap();
    assert!(store.inbox_rows(&a, None, None).await.unwrap().is_empty());
    for row in &rows {
        let raw = store.message(&a, &row.id).await.unwrap();
        assert_eq!(raw.folder_ids, vec!["INBOX"]);
        assert_eq!(
            store.visible_message(&a, raw).await.unwrap().folder_ids,
            vec!["Trash"]
        );
    }
    let job = store.sender_cleanup_jobs(&a).await.unwrap().remove(0);
    assert_eq!((job.pending, job.applied), (43, 0));
}
#[tokio::test]
async fn frozen_batch_is_atomic_durable_idempotent_and_reports_partial_outcomes() {
    let (dir, store, user, account, rows) = fixture().await;
    let a = store.authorize(&user, &account).await.unwrap();
    let request = batch(&rows);
    // An unreachable provider does not delay enqueue. No provider exists in Store.
    let started = std::time::Instant::now();
    let (one, two) = tokio::join!(
        store.enqueue_sender_cleanup(&a, request.clone()),
        store.enqueue_sender_cleanup(&a, request.clone())
    );
    assert_eq!(one.unwrap().pending, 43);
    assert_eq!(two.unwrap().pending, 43);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let mut changed = request.clone();
    changed.search_complete = true;
    assert!(matches!(
        store.enqueue_sender_cleanup(&a, changed).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        store.enqueue_sender_cleanup(&a, batch(&rows)).await,
        Err(Error::Conflict)
    ));
    assert!(store.dismiss_sender_cleanup(&a, &request.id).await.is_err());
    // Freeze excludes arrivals discovered after the deliberate click.
    let mut late = rows[0].clone();
    late.id = "late".into();
    store.upsert_message(&a, late).await.unwrap();
    let mut first = store.claim_mutation(&a).await.unwrap().unwrap();
    first.status = OperationStatus::Applied;
    store
        .finish_mutation(&a, first, Some(vec!["Trash".into()]))
        .await
        .unwrap();
    let uncertain = store.claim_mutation(&a).await.unwrap().unwrap();
    // Simulate a restart after provider work started: never replay this move.
    drop(store);
    let store = Store::open(dir.path(), [42; 32]).await.unwrap();
    let a = store.authorize(&user, &account).await.unwrap();
    store.recover_mutations(&a).await.unwrap();
    let job = store.sender_cleanup_jobs(&a).await.unwrap().pop().unwrap();
    assert_eq!((job.applied, job.failed, job.pending), (1, 1, 41));
    assert_eq!(job.failed_targets, vec![uncertain.target_id]);
    let retried = store
        .enqueue_sender_cleanup(&a, request.clone())
        .await
        .unwrap();
    assert_eq!(
        (retried.applied, retried.failed, retried.pending),
        (1, 1, 41)
    );
    let mut claims = 0;
    while let Some(mut op) = store.claim_mutation(&a).await.unwrap() {
        assert_ne!(op.target_id, "late");
        op.status = OperationStatus::Applied;
        store
            .finish_mutation(&a, op, Some(vec!["Trash".into()]))
            .await
            .unwrap();
        claims += 1;
    }
    assert_eq!(claims, 41);
    let job = store.sender_cleanup_jobs(&a).await.unwrap().pop().unwrap();
    assert_eq!((job.applied, job.failed, job.pending), (42, 1, 0));
    store.dismiss_sender_cleanup(&a, &request.id).await.unwrap();
    assert!(store.sender_cleanup_jobs(&a).await.unwrap().is_empty());
    assert!(matches!(
        store.enqueue_sender_cleanup(&a, batch(&rows)).await,
        Err(Error::Conflict)
    ));
    assert!(store.claim_mutation(&a).await.unwrap().is_none());
}
#[tokio::test]
async fn invalid_sender_stale_missing_duplicate_or_cross_account_targets_leave_no_partial_queue() {
    let (_dir, store, user, account, rows) = fixture().await;
    let a = store.authorize(&user, &account).await.unwrap();
    for mode in ["stale", "missing", "duplicate", "sender", "oversized"] {
        let mut request = batch(&rows);
        match mode {
            "stale" => request.targets.last_mut().unwrap().expected_revision = 999,
            "missing" => request.targets.last_mut().unwrap().id = "zzmissing".into(),
            "duplicate" => request.targets.push(request.targets[0].clone()),
            "oversized" => request.targets = vec![request.targets[0].clone(); 10001],
            "sender" => {
                let mut m = rows.last().unwrap().clone();
                m.sender.email = "xlist@example.invalid".into();
                store.upsert_message(&a, m).await.unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            store.enqueue_sender_cleanup(&a, request).await.is_err(),
            "{mode}"
        );
        assert!(store.sender_cleanup_jobs(&a).await.unwrap().is_empty());
        assert!(store.claim_mutation(&a).await.unwrap().is_none());
        assert_eq!(
            store.message(&a, &rows[0].id).await.unwrap().revision,
            rows[0].revision
        );
    }
    let other = store.user("issuer", "other").await.unwrap();
    assert!(store.authorize(&other, &account).await.is_err());
    let foreign = Store::open(tempfile::tempdir().unwrap().path(), [42; 32])
        .await
        .unwrap();
    assert!(foreign.sender_cleanup_jobs(&a).await.is_err());
}
