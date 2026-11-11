use dakia_domain::*;
use dakia_storage::Store;
async fn fixture() -> (
    tempfile::TempDir,
    Store,
    dakia_storage::AuthorizedAccount,
    Message,
) {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path(), [4; 32]).await.unwrap();
    let user = store.user("issuer", "owner").await.unwrap();
    let input=serde_json::from_value(serde_json::json!({"email":"test@example.invalid","display_name":"test","provider":"imap","imap_host":"imap.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.invalid","smtp_port":465,"smtp_tls":"implicit","username":"test","password":"fixture-only"})).unwrap();
    let account = store.connect(&user, &input).await.unwrap();
    let a = store.authorize(&user, &account.id).await.unwrap();
    let m:Message=serde_json::from_value(serde_json::json!({"id":"message","account_id":a.id(),"thread_id":"thread","provider_id":"stable","folder_ids":["INBOX"],"sender":{"email":"test@example.invalid","name":null},"to":[],"subject":"private fixture subject","preview":"","received_at":"2026-10-28T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    let m = store.upsert_message(&a, m).await.unwrap();
    (d, store, a, m)
}
#[tokio::test]
async fn stale_model_cannot_replace_manual_correction_and_restart_fails_running_jobs() {
    let (_d, s, a, m) = fixture().await;
    s.queue_model(&a, &m.id, JobKind::Classify, "model+policy2", None)
        .await
        .unwrap();
    let mut job = s.claim_model(&a).await.unwrap().unwrap();
    s.operation(
        &a,
        Operation {
            id: "manual".into(),
            account_id: a.id().into(),
            target_id: m.id.clone(),
            expected_revision: m.revision,
            change: Change::Category {
                value: "people".into(),
            },
            status: OperationStatus::Pending,
            revision: 0,
        },
    )
    .await
    .unwrap();
    job.status = JobStatus::Succeeded;
    s.finish_model(&a, job.clone(), Some("newsletters"), None)
        .await
        .unwrap();
    assert_eq!(
        s.message(&a, &m.id).await.unwrap().category.as_deref(),
        Some("people")
    );
    assert_eq!(
        s.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Failed
    );
    s.queue_model(&a, &m.id, JobKind::Translate, "revision", Some("et"))
        .await
        .unwrap();
    let job = s.claim_model(&a).await.unwrap().unwrap();
    s.recover_models(&a).await.unwrap();
    assert_eq!(
        s.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Failed
    );
}
#[tokio::test]
async fn initial_and_historical_mail_never_push_but_new_mail_deduplicates_and_wakes_snooze() {
    let (_d, s, a, m) = fixture().await;
    let subscription = serde_json::json!({"id":"endpoint","user_id":a.user_id(),"endpoint":"https://fcm.googleapis.com/synthetic-only","keys":{"p256dh":"fixture","auth":"fixture"}});
    s.subscribe(a.user_id(), "endpoint", &subscription.to_string())
        .await
        .unwrap();
    let other = s.user("issuer", "other").await.unwrap();
    let foreign = s.authorize(&other, a.id()).await;
    assert!(foreign.is_err());
    let (generation, _) = s.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let mut page = SearchPage {
        rows: vec![m.clone()],
        continuation: None,
        coverage: vec![AccountCoverage {
            account_id: a.id().into(),
            folder_ids: vec!["INBOX".into()],
            inbox: None,
            state: CoverageState::Complete,
            error: None,
        }],
        classification: ClassificationCoverage {
            classified: 0,
            total: 1,
        },
        pending: false,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
    };
    s.record_scan(&a, "INBOX", generation, &page).await.unwrap();
    assert!(s.next_push(&a).await.unwrap().is_none());
    s.operation(
        &a,
        Operation {
            id: "snooze".into(),
            account_id: a.id().into(),
            target_id: m.id.clone(),
            expected_revision: m.revision,
            change: Change::Snooze {
                until: "2099-01-01T00:00:00Z".into(),
            },
            status: OperationStatus::Pending,
            revision: 0,
        },
    )
    .await
    .unwrap();
    s.reset_scan(&a, "INBOX").await.unwrap();
    let (generation, _) = s.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let mut old = m.clone();
    old.id = "historical".into();
    old.provider_id = "older".into();
    old = s.upsert_message(&a, old).await.unwrap();
    page.rows = vec![old];
    s.record_scan(&a, "INBOX", generation, &page).await.unwrap();
    assert!(s.next_push(&a).await.unwrap().is_none());
    assert!(s.is_snoozed(&a, &m.thread_id).await.unwrap());
    s.reset_scan(&a, "INBOX").await.unwrap();
    let (generation, _) = s.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let mut fresh = m.clone();
    fresh.id = "new-reply".into();
    fresh.provider_id = "new-provider".into();
    fresh.category = Some("people".into());
    fresh.category_status = CategoryStatus::Ready;
    fresh.received_at = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();
    fresh = s.upsert_message(&a, fresh).await.unwrap();
    page.rows = vec![fresh];
    s.record_scan(&a, "INBOX", generation, &page).await.unwrap();
    let job = s.next_push(&a).await.unwrap().unwrap();
    assert!(!job.to_string().contains("private fixture subject"));
    assert_eq!(job["scope"]["user_id"], a.user_id());
    assert!(!s.is_snoozed(&a, &m.thread_id).await.unwrap());
    s.finish_push(&a, "new-reply", "endpoint", None)
        .await
        .unwrap();
    // A terminal generation rejects late replay instead of moving freshness.
    assert!(matches!(
        s.record_scan(&a, "INBOX", generation, &page).await,
        Err(dakia_storage::Error::Conflict)
    ));
    s.reset_scan(&a, "INBOX").await.unwrap();
    let (generation, _) = s.scan_state(&a, "INBOX").await.unwrap().unwrap();
    s.record_scan(&a, "INBOX", generation, &page).await.unwrap();
    assert!(s.next_push(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn transient_model_fetch_failures_back_off_durably_and_have_a_finite_retry_budget() {
    use dakia_storage::ModelFailureReason as R;
    let (dir, s, a, m) = fixture().await;
    let queued = s
        .queue_model(&a, &m.id, JobKind::Classify, "model", None)
        .await
        .unwrap();
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    for attempt in 0..3 {
        let job = s.claim_model(&a).await.unwrap().unwrap();
        assert_eq!(job.id, queued.id);
        assert_eq!(
            s.defer_model_failure(&a, &job, R::ContentUnavailable)
                .await
                .unwrap(),
            attempt < 2
        );
        assert_eq!(
            s.model_failure_reason(&a, &job.id)
                .await
                .unwrap()
                .as_deref(),
            Some("content_unavailable")
        );
        assert!(s.claim_model(&a).await.unwrap().is_none());
        assert!(s.classification_candidate(&a).await.unwrap().is_none());
        if attempt < 2 {
            assert_eq!(
                s.message(&a, &m.id).await.unwrap().category_status,
                CategoryStatus::Pending
            );
            let delay: i64 =
                sqlx::query_scalar("SELECT retry_at-unixepoch('now') FROM model_jobs WHERE id=?")
                    .bind(&job.id)
                    .fetch_one(&db)
                    .await
                    .unwrap();
            assert!((if attempt == 0 { 28..=30 } else { 118..=120 }).contains(&delay));
            // Reopening/recovery cannot erase pending retry backoff.
            let reopened = Store::open(dir.path(), [4; 32]).await.unwrap();
            let auth = reopened.authorize(a.user_id(), a.id()).await.unwrap();
            reopened.recover_models(&auth).await.unwrap();
            assert!(reopened.claim_model(&auth).await.unwrap().is_none());
            sqlx::query("UPDATE model_jobs SET retry_at=0")
                .execute(&db)
                .await
                .unwrap();
        } else {
            let mut job = job;
            job.status = JobStatus::Failed;
            s.finish_model(&a, job, None, None).await.unwrap();
        }
    }
    assert_eq!(
        s.message(&a, &m.id).await.unwrap().category_status,
        CategoryStatus::Failed
    );
    assert!(s.classification_candidate(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn model_recovery_success_clears_reason_and_permanent_failures_or_expiry_do_not_retry() {
    use dakia_storage::ModelFailureReason as R;
    let (dir, s, a, m) = fixture().await;
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    for reason in [
        R::ContentInvalid,
        R::RevisionChanged,
        R::WorkerFailed,
        R::Deadline,
    ] {
        s.queue_model(&a, &m.id, JobKind::Classify, "model", None)
            .await
            .unwrap();
        let job = s.claim_model(&a).await.unwrap().unwrap();
        assert!(!s.defer_model_failure(&a, &job, reason).await.unwrap());
        let mut job = job;
        job.status = JobStatus::Succeeded;
        s.finish_model(&a, job.clone(), Some("people"), None)
            .await
            .unwrap();
        assert!(s.model_failure_reason(&a, &job.id).await.unwrap().is_none());
    }
    s.queue_model(&a, &m.id, JobKind::Classify, "model", None)
        .await
        .unwrap();
    let job = s.claim_model(&a).await.unwrap().unwrap();
    sqlx::query("UPDATE model_jobs SET document=json_set(document,'$.deadline','2000-01-01T00:00:00Z') WHERE id=?").bind(&job.id).execute(&db).await.unwrap();
    assert!(!s
        .defer_model_failure(&a, &job, R::ContentUnavailable)
        .await
        .unwrap());
}

#[tokio::test]
async fn upgrade_retries_legacy_failed_classification_once_without_touching_manual_categories() {
    let (dir, s, a, m) = fixture().await;
    let db = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}/accounts/{}.sqlite",
        dir.path().display(),
        a.id()
    ))
    .await
    .unwrap();
    sqlx::query(
        "UPDATE messages SET document=json_set(document,'$.category_status','failed') WHERE id=?",
    )
    .bind(&m.id)
    .execute(&db)
    .await
    .unwrap();
    let failed = s.message(&a, &m.id).await.unwrap();
    let mut manual = failed.clone();
    manual.id = "manual".into();
    manual.provider_id = "manual".into();
    s.upsert_message(&a, manual).await.unwrap();
    sqlx::query("UPDATE messages SET category_override='people' WHERE id='manual'")
        .execute(&db)
        .await
        .unwrap();
    // Reconstruct the deployed v14 schema, then run the actual migration.
    sqlx::raw_sql("DROP INDEX model_jobs_due; ALTER TABLE model_jobs DROP COLUMN retry_count; ALTER TABLE model_jobs DROP COLUMN retry_at; ALTER TABLE model_jobs DROP COLUMN failure_reason; DELETE FROM _sqlx_migrations WHERE version=15;").execute(&db).await.unwrap();
    let migrations = sqlx::migrate!("./migrations/account");
    migrations.run(&db).await.unwrap();
    let updated = s.message(&a, &m.id).await.unwrap();
    assert_eq!(updated.category_status, CategoryStatus::Pending);
    assert_eq!(updated.revision, failed.revision + 1);
    assert_eq!(
        s.message(&a, "manual").await.unwrap().category_status,
        CategoryStatus::Failed
    );
    // The migration ledger prevents repeated upgrades from requeueing failures.
    sqlx::query(
        "UPDATE messages SET document=json_set(document,'$.category_status','failed') WHERE id=?",
    )
    .bind(&m.id)
    .execute(&db)
    .await
    .unwrap();
    let updated = s.message(&a, &m.id).await.unwrap();
    migrations.run(&db).await.unwrap();
    assert_eq!(s.message(&a, &m.id).await.unwrap(), updated);
}

async fn set_read(s: &Store, a: &dakia_storage::AuthorizedAccount, id: &str, read: bool) {
    let m = s.message(a, id).await.unwrap();
    s.operation(
        a,
        Operation {
            id: format!("read-{}", m.revision),
            account_id: a.id().into(),
            target_id: id.into(),
            expected_revision: m.revision,
            change: Change::Read { value: read },
            status: OperationStatus::Pending,
            revision: 0,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn classification_only_selects_unread_and_queue_rechecks_read_state() {
    let (_d, s, a, m) = fixture().await;
    assert_eq!(
        s.classification_candidate(&a).await.unwrap().unwrap().id,
        m.id
    );
    // A read operation can race the candidate lookup.
    set_read(&s, &a, &m.id, true).await;
    assert!(s.classification_candidate(&a).await.unwrap().is_none());
    assert!(matches!(
        s.queue_model(&a, &m.id, JobKind::Classify, "model", None)
            .await,
        Err(dakia_storage::Error::Conflict)
    ));
    set_read(&s, &a, &m.id, false).await;
    assert_eq!(
        s.classification_candidate(&a).await.unwrap().unwrap().id,
        m.id
    );
    s.queue_model(&a, &m.id, JobKind::Classify, "model", None)
        .await
        .unwrap();
    let mut job = s.claim_model(&a).await.unwrap().unwrap();
    job.status = JobStatus::Succeeded;
    s.finish_model(&a, job, Some("people"), None).await.unwrap();
    set_read(&s, &a, &m.id, true).await;
    set_read(&s, &a, &m.id, false).await;
    assert_eq!(
        s.message(&a, &m.id).await.unwrap().category.as_deref(),
        Some("people")
    );
    assert!(s.classification_candidate(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn queued_read_classification_is_cancelled_without_blocking_translation() {
    let (_d, s, a, m) = fixture().await;
    let job = s
        .queue_model(&a, &m.id, JobKind::Classify, "model", None)
        .await
        .unwrap();
    set_read(&s, &a, &m.id, true).await;
    let before = s.message(&a, &m.id).await.unwrap();
    let translation = s
        .queue_model(&a, &m.id, JobKind::Translate, "model", Some("et"))
        .await
        .unwrap();
    assert_eq!(s.claim_model(&a).await.unwrap().unwrap().id, translation.id);
    assert_eq!(
        s.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Cancelled
    );
    assert!(s.model_failure_reason(&a, &job.id).await.unwrap().is_none());
    assert_eq!(s.message(&a, &m.id).await.unwrap(), before);
}

#[tokio::test]
async fn reading_cancels_delayed_classification_retries_across_restart() {
    use dakia_storage::ModelFailureReason as R;
    let (d, s, a, m) = fixture().await;
    s.queue_model(&a, &m.id, JobKind::Classify, "model", None)
        .await
        .unwrap();
    let job = s.claim_model(&a).await.unwrap().unwrap();
    assert!(s
        .defer_model_failure(&a, &job, R::ContentUnavailable)
        .await
        .unwrap());
    set_read(&s, &a, &m.id, true).await;
    let reopened = Store::open(d.path(), [4; 32]).await.unwrap();
    let auth = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    reopened.recover_models(&auth).await.unwrap();
    assert!(reopened.claim_model(&auth).await.unwrap().is_none());
    assert_eq!(
        reopened.model_job(&auth, &job.id).await.unwrap().status,
        JobStatus::Cancelled
    );
    assert!(reopened
        .model_failure_reason(&auth, &job.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        reopened
            .message(&auth, &m.id)
            .await
            .unwrap()
            .category_status,
        CategoryStatus::Pending
    );
    set_read(&s, &a, &m.id, false).await;
    assert!(s.classification_candidate(&a).await.unwrap().is_some());
}

#[tokio::test]
async fn reading_during_classification_prevents_retry_and_late_category_commit() {
    use dakia_storage::ModelFailureReason as R;
    for succeeds in [true, false] {
        let (_d, s, a, m) = fixture().await;
        s.queue_model(&a, &m.id, JobKind::Classify, "model", None)
            .await
            .unwrap();
        let mut job = s.claim_model(&a).await.unwrap().unwrap();
        set_read(&s, &a, &m.id, true).await;
        let before = s.message(&a, &m.id).await.unwrap();
        assert!(!s
            .defer_model_failure(&a, &job, R::ContentUnavailable)
            .await
            .unwrap());
        job.status = if succeeds {
            JobStatus::Succeeded
        } else {
            JobStatus::Failed
        };
        s.finish_model(&a, job.clone(), succeeds.then_some("people"), None)
            .await
            .unwrap();
        assert_eq!(
            s.model_job(&a, &job.id).await.unwrap().status,
            JobStatus::Cancelled
        );
        assert!(s.model_failure_reason(&a, &job.id).await.unwrap().is_none());
        assert_eq!(s.message(&a, &m.id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn restart_cancels_read_classification_that_was_running() {
    let (_d, s, a, m) = fixture().await;
    s.queue_model(&a, &m.id, JobKind::Classify, "model", None)
        .await
        .unwrap();
    let job = s.claim_model(&a).await.unwrap().unwrap();
    set_read(&s, &a, &m.id, true).await;
    let before = s.message(&a, &m.id).await.unwrap();
    s.recover_models(&a).await.unwrap();
    assert_eq!(
        s.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Cancelled
    );
    assert!(s.model_failure_reason(&a, &job.id).await.unwrap().is_none());
    assert_eq!(s.message(&a, &m.id).await.unwrap(), before);
}

#[tokio::test]
async fn snooze_metadata_counts_thread_messages_and_only_successful_inbox_restore_clears_it() {
    let (dir, s, a, m) = fixture().await;
    let mut sibling = m.clone(); sibling.id = "sibling".into(); sibling.provider_id = "second".into();
    s.upsert_message(&a, sibling).await.unwrap();
    let until = "2099-01-01T09:00:00Z";
    s.operation(&a, Operation { id: "snooze-metadata".into(), account_id: a.id().into(), target_id: m.id.clone(), expected_revision: m.revision,
        change: Change::Snooze { until: until.into() }, status: OperationStatus::Pending, revision: 0 }).await.unwrap();
    let rows = s.active_snoozes(&a).await.unwrap();
    assert_eq!(rows.len(), 1); assert_eq!(rows[0]["message_count"], 2);
    assert_eq!(rows[0]["thread_id"], m.thread_id);
    assert_eq!(rows[0]["until"], "2099-01-01T09:00:00.000Z");
    assert!(s.inbox_rows(&a, None, None).await.unwrap().is_empty());
    let mut archived = s.message(&a, &m.id).await.unwrap();
    archived.folder_ids = vec!["Archive".into()];
    let current = s.upsert_message(&a, archived).await.unwrap();
    let pending = s.operation(&a, Operation { id: "restore-failed".into(), account_id: a.id().into(), target_id: m.id.clone(), expected_revision: current.revision,
        change: Change::Move { folder_id: "INBOX".into() }, status: OperationStatus::Pending, revision: 0 }).await.unwrap();
    let mut failed = pending; failed.status = OperationStatus::Failed;
    s.finish_mutation(&a, failed, None).await.unwrap();
    assert_eq!(s.active_snoozes(&a).await.unwrap().len(), 1, "a failed restore must not unsnooze");
    let current = s.message(&a, &m.id).await.unwrap();
    let mut applied = s.operation(&a, Operation { id: "restore-success".into(), account_id: a.id().into(), target_id: m.id.clone(), expected_revision: current.revision,
        change: Change::Move { folder_id: "INBOX".into() }, status: OperationStatus::Pending, revision: 0 }).await.unwrap();
    applied.status = OperationStatus::Applied;
    s.finish_mutation(&a, applied, Some(vec!["INBOX".into()])).await.unwrap();
    assert!(s.active_snoozes(&a).await.unwrap().is_empty());
    assert_eq!(s.inbox_rows(&a, None, None).await.unwrap().len(), 2);
    let reopened = Store::open(dir.path(), [4;32]).await.unwrap();
    let a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    assert!(reopened.active_snoozes(&a).await.unwrap().is_empty());
    assert_eq!(reopened.inbox_rows(&a, None, None).await.unwrap().len(), 2);
}

#[tokio::test]
async fn restoring_a_snoozed_inbox_member_does_not_copy_it_to_the_same_provider_folder() {
    let (_dir, s, a, m) = fixture().await;
    let snoozed = s.operation(&a, Operation { id: "local-snooze".into(), account_id: a.id().into(), target_id: m.id.clone(), expected_revision: m.revision,
        change: Change::Snooze { until: "2099-01-01T09:00:00Z".into() }, status: OperationStatus::Pending, revision: 0 }).await.unwrap();
    let restored = s.operation(&a, Operation { id: "local-restore".into(), account_id: a.id().into(), target_id: m.id.clone(), expected_revision: snoozed.revision,
        change: Change::Move { folder_id: "INBOX".into() }, status: OperationStatus::Pending, revision: 0 }).await.unwrap();
    assert_eq!(restored.status, OperationStatus::Applied);
    let repeated = s.operation(&a, Operation { id: "already-restored".into(), account_id: a.id().into(), target_id: m.id.clone(), expected_revision: restored.revision,
        change: Change::Move { folder_id: "INBOX".into() }, status: OperationStatus::Pending, revision: 0 }).await.unwrap();
    assert_eq!(repeated.status, OperationStatus::Applied, "repeat and same-thread bulk restores must not queue a copy to Inbox");
    assert!(s.claim_mutation(&a).await.unwrap().is_none());
    assert!(s.active_snoozes(&a).await.unwrap().is_empty());
    assert_eq!(s.inbox_rows(&a, None, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn inbox_restore_waits_for_an_earlier_archive() {
    for archive_running in [false, true] {
        let (_dir, s, a, m) = fixture().await;
        let archive = s
            .operation(
                &a,
                Operation {
                    id: "earlier-archive".into(),
                    account_id: a.id().into(),
                    target_id: m.id.clone(),
                    expected_revision: m.revision,
                    change: Change::Archive,
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
        if archive_running {
            assert_eq!(s.claim_mutation(&a).await.unwrap().unwrap().id, archive.id);
        }
        let restore = s
            .operation(
                &a,
                Operation {
                    id: "later-restore".into(),
                    account_id: a.id().into(),
                    target_id: m.id.clone(),
                    expected_revision: archive.revision,
                    change: Change::Move {
                        folder_id: "INBOX".into(),
                    },
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(restore.status, OperationStatus::Pending);
        let mut archive = if archive_running {
            archive
        } else {
            s.claim_mutation(&a).await.unwrap().unwrap()
        };
        assert_eq!(archive.id, "earlier-archive");
        assert!(s.claim_mutation(&a).await.unwrap().is_none());
        archive.status = OperationStatus::Applied;
        s.finish_mutation(&a, archive, Some(vec!["Archive".into()]))
            .await
            .unwrap();
        let mut restore = s.claim_mutation(&a).await.unwrap().unwrap();
        assert_eq!(restore.id, "later-restore");
        restore.status = OperationStatus::Applied;
        s.finish_mutation(&a, restore, Some(vec!["INBOX".into()]))
            .await
            .unwrap();
        assert_eq!(
            s.message(&a, &m.id).await.unwrap().folder_ids,
            vec!["INBOX"]
        );
        assert!(s.claim_mutation(&a).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn finishing_an_older_restore_preserves_a_newer_thread_snooze() {
    for same_deadline in [false, true] {
        for snooze_sibling in [false, true] {
            let (dir, s, a, mut m) = fixture().await;
            m.folder_ids = vec!["Archive".into()];
            let m = s.upsert_message(&a, m).await.unwrap();
            let mut sibling = m.clone();
            sibling.id = "sibling".into();
            sibling.provider_id = "second".into();
            s.upsert_message(&a, sibling).await.unwrap();
            let original_deadline = "2099-01-01T09:00:00Z";
            let snooze = s
                .operation(
                    &a,
                    Operation {
                        id: "original-snooze".into(),
                        account_id: a.id().into(),
                        target_id: m.id.clone(),
                        expected_revision: m.revision,
                        change: Change::Snooze {
                            until: original_deadline.into(),
                        },
                        status: OperationStatus::Pending,
                        revision: 0,
                    },
                )
                .await
                .unwrap();
            s.operation(
                &a,
                Operation {
                    id: "older-restore".into(),
                    account_id: a.id().into(),
                    target_id: m.id.clone(),
                    expected_revision: snooze.revision,
                    change: Change::Move {
                        folder_id: "INBOX".into(),
                    },
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
            let mut restore = s.claim_mutation(&a).await.unwrap().unwrap();
            let target = s
                .message(&a, if snooze_sibling { "sibling" } else { &m.id })
                .await
                .unwrap();
            let until = if same_deadline {
                original_deadline
            } else {
                "2099-02-01T09:00:00Z"
            };
            s.operation(
                &a,
                Operation {
                    id: "newer-snooze".into(),
                    account_id: a.id().into(),
                    target_id: target.id,
                    expected_revision: target.revision,
                    change: Change::Snooze {
                        until: until.into(),
                    },
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
            let before = s.active_snoozes(&a).await.unwrap();
            let reopened = Store::open(dir.path(), [4; 32]).await.unwrap();
            let a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
            restore.status = OperationStatus::Applied;
            reopened
                .finish_mutation(&a, restore, Some(vec!["INBOX".into()]))
                .await
                .unwrap();
            assert_eq!(reopened.active_snoozes(&a).await.unwrap(), before);
            assert!(reopened
                .inbox_rows(&a, None, None)
                .await
                .unwrap()
                .is_empty());
        }
    }
}
