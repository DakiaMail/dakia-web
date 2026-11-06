use dakia_domain::*;
use dakia_storage::{CleanupLocation, CleanupRequest, CleanupTarget, Error, Store};

async fn fixture(count: usize) -> (tempfile::TempDir, Store, String, String, Vec<Message>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [42; 32]).await.unwrap();
    let user = store.user("issuer", "owner").await.unwrap();
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"fixture@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let mut rows = vec![];
    for n in 0..count {
        let m = serde_json::from_value(serde_json::json!({"id":format!("m{n:03}"),"account_id":a.id(),"thread_id":"fixture","provider_id":format!("fixture-{n}"),"folder_ids":["INBOX"],"sender":{"email":"list@example.invalid","name":null},"to":[],"subject":"Synthetic list","preview":"Fixture","received_at":"2026-01-01T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
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
                expected_location: Some(CleanupLocation {
                    provider_id: m.provider_id.clone(),
                    folder_ids: m.folder_ids.clone(),
                    sender_email: m.sender.email.clone(),
                }),
            })
            .collect(),
    }
}
#[tokio::test]
async fn cleanup_survives_reader_read_classification_and_refresh_drift_at_multiple_counts() {
    for count in [1, 2, 43, 125] {
        let (dir, store, user, account, rows) = fixture(count).await;
        let a = store.authorize(&user, &account).await.unwrap();
        let request = batch(&rows);
        // A harmless classification refresh advances the revision after capture.
        for row in &rows {
            let mut changed = row.clone();
            changed.category = Some("newsletters".into());
            changed.category_status = CategoryStatus::Ready;
            store.upsert_message(&a, changed).await.unwrap();
        }
        let source = store.message(&a, &rows[0].id).await.unwrap();
        store
            .operation(
                &a,
                Operation {
                    id: "mark-read".into(),
                    account_id: account.clone(),
                    target_id: source.id,
                    expected_revision: source.revision,
                    change: Change::Read { value: true },
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
        let mut legacy = request.clone();
        for target in &mut legacy.targets {
            target.expected_location = None;
        }
        assert!(matches!(
            store.enqueue_sender_cleanup(&a, legacy).await,
            Err(Error::Conflict)
        ));
        assert!(store.sender_cleanup_jobs(&a).await.unwrap().is_empty());
        assert_eq!(
            store
                .enqueue_sender_cleanup(&a, request.clone())
                .await
                .unwrap()
                .pending,
            count
        );
        let mut read = store.claim_mutation(&a).await.unwrap().unwrap();
        assert!(matches!(read.change, Change::Read { .. }));
        read.status = OperationStatus::Applied;
        store
            .finish_mutation(&a, read, Some(vec!["INBOX".into()]))
            .await
            .unwrap();
        drop(store);
        let store = Store::open(dir.path(), [42; 32]).await.unwrap();
        let a = store.authorize(&user, &account).await.unwrap();
        assert_eq!(
            store
                .enqueue_sender_cleanup(&a, request)
                .await
                .unwrap()
                .pending,
            count
        );
        let mut moved = std::collections::HashSet::new();
        while let Some(mut op) = store.claim_mutation(&a).await.unwrap() {
            assert_eq!(op.change, Change::Trash);
            assert!(moved.insert(op.target_id.clone()));
            op.status = OperationStatus::Applied;
            store
                .finish_mutation(&a, op, Some(vec!["Trash".into()]))
                .await
                .unwrap();
        }
        assert_eq!(moved.len(), count);
        assert_eq!(
            store.sender_cleanup_jobs(&a).await.unwrap()[0].applied,
            count
        );
    }
}
#[tokio::test]
async fn cleanup_location_guard_rejects_moves_identity_changes_and_uncertain_or_pending_moves_atomically(
) {
    for mode in [
        "folders",
        "identity",
        "sender",
        "future",
        "empty",
        "pending",
        "uncertain",
    ] {
        let (_dir, store, user, account, rows) = fixture(2).await;
        let a = store.authorize(&user, &account).await.unwrap();
        let mut request = batch(&rows);
        let mut changed = rows[1].clone();
        match mode {
            "folders" => changed.folder_ids = vec!["Trash".into()],
            "identity" => changed.provider_id = "replacement".into(),
            "sender" => changed.sender.email = "other@example.invalid".into(),
            "future" => request.targets[1].expected_revision = 999,
            "empty" => request.targets[1]
                .expected_location
                .as_mut()
                .unwrap()
                .folder_ids
                .clear(),
            "pending" | "uncertain" => {
                store
                    .operation(
                        &a,
                        Operation {
                            id: "existing-move".into(),
                            account_id: account.clone(),
                            target_id: changed.id.clone(),
                            expected_revision: changed.revision,
                            change: Change::Archive,
                            status: OperationStatus::Pending,
                            revision: 0,
                        },
                    )
                    .await
                    .unwrap();
                if mode == "uncertain" {
                    let mut op = store.claim_mutation(&a).await.unwrap().unwrap();
                    op.status = OperationStatus::Failed;
                    store.finish_mutation(&a, op, None).await.unwrap();
                }
            }
            _ => unreachable!(),
        }
        if matches!(mode, "folders" | "identity" | "sender") {
            store.upsert_message(&a, changed).await.unwrap();
        }
        let error = store
            .enqueue_sender_cleanup(&a, request)
            .await
            .err()
            .unwrap();
        if matches!(mode, "pending" | "uncertain") {
            assert!(matches!(error, Error::CleanupMoveUnverified), "{mode}");
        }
        assert!(store.sender_cleanup_jobs(&a).await.unwrap().is_empty());
        assert_eq!(
            store.message(&a, &rows[0].id).await.unwrap().revision,
            rows[0].revision,
            "atomic rollback: {mode}"
        );
    }
}
