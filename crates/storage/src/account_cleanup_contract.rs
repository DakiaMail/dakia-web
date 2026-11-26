use super::*;
fn target(message: &Message) -> CleanupTarget {
    CleanupTarget {
        id: message.id.clone(),
        expected_revision: message.revision,
        expected_location: Some(CleanupLocation {
            provider_id: message.provider_id.clone(),
            folder_ids: message.folder_ids.clone(),
            sender_email: message.sender.email.clone(),
        }),
    }
}
fn request(source: &Message, targets: Vec<CleanupTarget>) -> CleanupRequest {
    CleanupRequest {
        id: Uuid::new_v4().to_string(),
        source_id: source.id.clone(),
        targets,
        search_complete: true,
    }
}
pub(super) async fn contract(a: &AccountRepository, b: &AccountRepository) {
    for _ in 0..30 {
        let Some(mut operation) = a.claim_mutation(&[]).await.unwrap() else {
            break;
        };
        operation.status = OperationStatus::Failed;
        a.finish_mutation(operation, None).await.unwrap();
    }
    assert!(a.claim_mutation(&[]).await.unwrap().is_none());
    let rows = a
        .ingest_messages(
            vec![
                message(&a.account, "cleanup-source"),
                message(&a.account, "cleanup-one"),
                message(&a.account, "cleanup-two"),
            ],
            &[],
            &[],
            None,
        )
        .await
        .unwrap();
    let frozen = request(&rows[0], vec![target(&rows[2]), target(&rows[1])]);
    a.operation(mutation(
        &rows[1],
        "cleanup-flag",
        Change::Star { value: true },
    ))
    .await
    .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let (a, frozen) = (a.clone(), frozen.clone());
        tasks.spawn(async move { a.enqueue_sender_cleanup(frozen).await.unwrap() });
    }
    while let Some(result) = tasks.join_next().await {
        let job = result.unwrap();
        assert_eq!(job.request.id, frozen.id);
        assert_eq!(job.pending, 2);
        assert_eq!(job.account_id, a.account);
    }
    assert_eq!(a.message("cleanup-one").await.unwrap().revision, 3);
    assert_eq!(a.message("cleanup-two").await.unwrap().revision, 2);
    let mut changed = frozen.clone();
    changed.search_complete = false;
    assert!(matches!(
        a.enqueue_sender_cleanup(changed).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        a.dismiss_sender_cleanup(&frozen.id).await,
        Err(Error::Conflict)
    ));
    let jobs = a.sender_cleanup_jobs().await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].projected_rows.len(), 2);
    assert!(jobs[0]
        .projected_rows
        .iter()
        .all(|m| m.folder_ids == ["Trash"]));
    assert_eq!(
        a.message("cleanup-one").await.unwrap().folder_ids,
        ["INBOX"]
    );
    assert!(b.sender_cleanup_jobs().await.unwrap().is_empty());
    let mut completed = 0;
    for _ in 0..5 {
        let Some(mut operation) = a.claim_mutation(&[]).await.unwrap() else {
            break;
        };
        let folders = if operation.change == Change::Trash && operation.target_id == "cleanup-one" {
            operation.status = OperationStatus::Applied;
            Some(vec!["Deleted Items".into()])
        } else if operation.change == Change::Trash {
            operation.status = OperationStatus::Failed;
            None
        } else {
            operation.status = OperationStatus::Applied;
            None
        };
        a.finish_mutation(operation, folders).await.unwrap();
        completed += 1;
    }
    assert_eq!(completed, 3);
    let jobs = a.sender_cleanup_jobs().await.unwrap();
    assert_eq!(
        (jobs[0].applied, jobs[0].failed, jobs[0].pending),
        (1, 1, 0)
    );
    assert_eq!(jobs[0].failed_targets, ["cleanup-two"]);
    a.dismiss_sender_cleanup(&frozen.id).await.unwrap();
    assert!(a.sender_cleanup_jobs().await.unwrap().is_empty());
    let before = a.message("cleanup-one").await.unwrap().revision;
    let replay = a.enqueue_sender_cleanup(frozen.clone()).await.unwrap();
    assert!(replay.dismissed);
    assert_eq!(replay.applied, 1);
    assert_eq!(a.message("cleanup-one").await.unwrap().revision, before);
    let mut overlap = frozen;
    overlap.id = Uuid::new_v4().to_string();
    assert!(matches!(
        a.enqueue_sender_cleanup(overlap).await,
        Err(Error::Conflict)
    ));

    let mut restored = a.message("cleanup-one").await.unwrap();
    restored.folder_ids = vec!["INBOX".into()];
    let restored = a
        .ingest_messages(vec![restored], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    let repeated = a
        .enqueue_sender_cleanup(request(&restored, vec![target(&restored)]))
        .await
        .unwrap();
    assert_eq!(
        repeated.pending, 1,
        "a restored email can be cleaned up again"
    );
    let mut move_again = a.claim_mutation(&[]).await.unwrap().unwrap();
    assert_eq!(move_again.target_id, restored.id);
    move_again.status = OperationStatus::Applied;
    a.finish_mutation(move_again, Some(vec!["Deleted Items".into()]))
        .await
        .unwrap();

    let mut wrong = message(&a.account, "cleanup-rollback-z");
    wrong.sender.email = "another@example.invalid".into();
    let rows = a
        .ingest_messages(
            vec![message(&a.account, "cleanup-rollback-a"), wrong],
            &[],
            &[],
            None,
        )
        .await
        .unwrap();
    let failed = request(&rows[0], rows.iter().map(target).collect());
    let cursor = a.event_cursor().await.unwrap();
    let before = receipts(a).await;
    assert!(matches!(
        a.enqueue_sender_cleanup(failed.clone()).await,
        Err(Error::Invalid)
    ));
    assert_eq!(a.message("cleanup-rollback-a").await.unwrap().revision, 1);
    assert!(a
        .provider_document(&format!("sender-cleanup:{}", failed.id))
        .await
        .unwrap()
        .is_none());
    assert_eq!(a.event_cursor().await.unwrap(), cursor);
    assert_eq!(receipts(a).await, before);
    assert!(a.claim_mutation(&[]).await.unwrap().is_none());
    let row = a
        .ingest_messages(
            vec![message(&a.account, "cleanup-location")],
            &[],
            &[],
            None,
        )
        .await
        .unwrap()
        .remove(0);
    let frozen = request(&row, vec![target(&row)]);
    let mut moved = row;
    moved.folder_ids = vec!["Elsewhere".into()];
    a.ingest_messages(vec![moved], &[], &[], None)
        .await
        .unwrap();
    assert!(matches!(
        a.enqueue_sender_cleanup(frozen).await,
        Err(Error::Conflict)
    ));
    catalogue(a, b).await;
}
async fn catalogue(a: &AccountRepository, b: &AccountRepository) {
    let mut rows = Vec::new();
    for (id, date, folders) in [
        ("catalogue-a", "2030-11-10T00:00:00Z", vec!["INBOX".into()]),
        (
            "catalogue-b",
            "2030-11-10T02:00:00+02:00",
            vec!["INBOX".into()],
        ),
        (
            "catalogue-late",
            "2030-11-11T00:00:00Z",
            vec!["INBOX".into()],
        ),
        (
            "catalogue-custom",
            "2030-11-10T00:00:00Z",
            vec!["Deleted Items".into()],
        ),
        (
            "catalogue-trash",
            "2030-11-10T00:00:00Z",
            vec![format!("{}Trash", char::from(92))],
        ),
        ("catalogue-without-folder", "2030-11-10T00:00:00Z", vec![]),
        (
            "catalogue-pending",
            "2030-11-10T00:00:00Z",
            vec!["INBOX".into()],
        ),
        ("catalogue-invalid", "invalid-date", vec!["INBOX".into()]),
    ] {
        let mut row = message(&a.account, id);
        row.sender.email = if id == "catalogue-b" {
            " Équipe@example.invalid "
        } else {
            "équipe@example.invalid"
        }
        .into();
        row.received_at = date.into();
        row.folder_ids = folders;
        rows.push(row);
    }
    a.ingest_messages(rows, &[], &[], None).await.unwrap();
    // Simulate an existing row whose original address predates the sender key.
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE messages SET sender_key=NULL WHERE id='catalogue-b'")
                .execute(db)
                .await
                .unwrap();
            crate::sender_key::backfill_sqlite(db).await.unwrap();
        }
        Backend::Postgres(db) => {
            let mut tx = db.begin().await.unwrap();
            sqlx::query(
                "UPDATE messages SET sender_key=NULL WHERE account_id=$1 AND id='catalogue-b'",
            )
            .bind(&a.account)
            .execute(&mut *tx)
            .await
            .unwrap();
            crate::sender_key::backfill_postgres(&mut tx, None)
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
    }
    let pending = a.message("catalogue-pending").await.unwrap();
    a.operation(mutation(&pending, "catalogue-move", Change::Trash))
        .await
        .unwrap();
    let (count, rows) = a
        .cached_sender_mail(
            "équipe@example.invalid",
            "2030-11-10T00:00:00Z",
            None,
            &["Deleted Items".into()],
        )
        .await
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        rows.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["catalogue-a", "catalogue-b"]
    );
    let (count, rows) = a
        .cached_sender_mail(
            "équipe@example.invalid",
            "2030-11-10T00:00:00Z",
            Some("catalogue-a"),
            &["Deleted Items".into()],
        )
        .await
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "catalogue-b");
    assert_eq!(
        a.cached_sender_mail("ÉQUIPE@example.invalid", "2030-11-10T00:00:00Z", None, &[],)
            .await
            .unwrap()
            .0,
        3
    );
    assert_eq!(
        a.cached_sender_mail("équipe@example.invalid", "invalid-cutoff", None, &[],)
            .await
            .unwrap()
            .0,
        0
    );
    assert_eq!(
        b.cached_sender_mail("équipe@example.invalid", "2030-11-10T00:00:00Z", None, &[],)
            .await
            .unwrap()
            .0,
        0
    );
}
