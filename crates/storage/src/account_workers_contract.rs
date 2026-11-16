use super::*;

pub(super) async fn contract(a: &AccountRepository, b: &AccountRepository) {
    models(a, b).await;
    notifications(a, b).await;
    references(a, b).await;
}

async fn body_state(
    repository: &AccountRepository,
    id: &str,
    state: Option<&str>,
) -> (String, i64) {
    match &repository.backend {
        Backend::Sqlite(db) => {
            if let Some(state) = state {
                sqlx::query("UPDATE search_documents SET body_state=? WHERE message_id=?")
                    .bind(state)
                    .bind(id)
                    .execute(db)
                    .await
                    .unwrap();
            }
            sqlx::query_as("SELECT body_state,retry_at FROM search_documents WHERE message_id=?")
                .bind(id)
                .fetch_one(db)
                .await
                .unwrap()
        }
        Backend::Postgres(db) => {
            if let Some(state) = state {
                sqlx::query("UPDATE search_documents SET body_state=$1 WHERE account_id=$2 AND message_id=$3").bind(state).bind(&repository.account).bind(id).execute(db).await.unwrap();
            }
            sqlx::query_as("SELECT body_state,retry_at FROM search_documents WHERE account_id=$1 AND message_id=$2").bind(&repository.account).bind(id).fetch_one(db).await.unwrap()
        }
    }
}
async fn reference_failure(repository: &AccountRepository, enabled: bool) {
    match &repository.backend {
        Backend::Sqlite(db) => {
            if enabled {
                sqlx::query("CREATE TRIGGER reference_failure BEFORE UPDATE OF document ON provider_documents WHEN NEW.key='content:references:reference-worker' AND NEW.document<>OLD.document BEGIN SELECT RAISE(ABORT,'fixture failure'); END").execute(db).await.unwrap();
            } else {
                sqlx::query("DROP TRIGGER reference_failure")
                    .execute(db)
                    .await
                    .unwrap();
            }
        }
        Backend::Postgres(db) => {
            if enabled {
                sqlx::query("CREATE FUNCTION reference_failure() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN IF NEW.key='content:references:reference-worker' AND NEW.document<>OLD.document THEN RAISE EXCEPTION 'fixture failure'; END IF; RETURN NEW; END $$").execute(db).await.unwrap();
                sqlx::query("CREATE TRIGGER reference_failure BEFORE UPDATE OF document ON provider_documents FOR EACH ROW EXECUTE FUNCTION reference_failure()").execute(db).await.unwrap();
            } else {
                sqlx::query("DROP TRIGGER reference_failure ON provider_documents")
                    .execute(db)
                    .await
                    .unwrap();
                sqlx::query("DROP FUNCTION reference_failure()")
                    .execute(db)
                    .await
                    .unwrap();
            }
        }
    }
}
async fn references(a: &AccountRepository, b: &AccountRepository) {
    assert!(a.claim_reference_recovery().await.unwrap().is_none());
    assert!(a
        .provider_document("content:recovery_due")
        .await
        .unwrap()
        .is_none());
    let mut row = message(&a.account, "reference-worker");
    row.provider_id = "gmail:reference-worker".into();
    let row = a
        .ingest_messages(vec![row], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    cached_search_document(a, &row).await;
    body_state(a, &row.id, Some("pending")).await;
    a.put_provider_document("reference:reference-worker", "reference-one")
        .await
        .unwrap();
    let state = a.content_references(&row).await.unwrap();
    assert!(matches!(
        b.content_references(&row).await,
        Err(Error::Invalid)
    ));
    assert!(!a
        .block_content_reference(&row, "reference-one", "wrong-token")
        .await
        .unwrap());
    assert!(a
        .block_content_reference(&row, "reference-one", &state.token)
        .await
        .unwrap());
    assert!(a.mark_reference_gap(&row, &state.token).await.unwrap());
    assert_eq!(
        body_state(a, &row.id, None).await.0,
        "reference_unavailable"
    );
    assert_eq!(
        a.claim_reference_recovery().await.unwrap().unwrap().id,
        row.id
    );
    let due = body_state(a, &row.id, None).await.1;
    assert!(due > Utc::now().timestamp() + 86000);
    for _ in 0..3 {
        assert!(a.mark_reference_gap(&row, &state.token).await.unwrap());
    }
    assert_eq!(body_state(a, &row.id, None).await.1, due);
    a.put_provider_document("content:recovery_due", "0")
        .await
        .unwrap();
    assert!(a.claim_reference_recovery().await.unwrap().is_none());
    let blocked = a.content_references(&row).await.unwrap();
    let docs = vec![
        ("reference:reference-worker".into(), "reference-one".into()),
        (
            "__content_observation:reference-worker".into(),
            (blocked.blocked_at + 1).to_string(),
        ),
    ];
    a.ingest_messages(vec![row.clone()], &docs, &[], None)
        .await
        .unwrap();
    let fresh = a.content_references(&row).await.unwrap();
    assert_ne!(fresh.token, state.token);
    assert!(fresh.blocked.is_empty());
    assert!(!a.mark_reference_gap(&row, &state.token).await.unwrap());
    assert_eq!(body_state(a, &row.id, None).await, ("pending".into(), 0));
    a.forget_empty_content_references(&row.id, "wrong-token")
        .await
        .unwrap();
    assert!(a
        .provider_document("content:references:reference-worker")
        .await
        .unwrap()
        .is_some());
    a.forget_empty_content_references(&row.id, &fresh.token)
        .await
        .unwrap();
    assert!(a
        .provider_document("content:references:reference-worker")
        .await
        .unwrap()
        .is_none());
    let state = a.content_references(&row).await.unwrap();
    for reference in ["reference-one", "reference-two"] {
        assert!(a
            .block_content_reference(&row, reference, &state.token)
            .await
            .unwrap());
    }
    reference_failure(a, true).await;
    assert!(a
        .promote_content_reference(&row, "reference-two", &state.token)
        .await
        .is_err());
    reference_failure(a, false).await;
    assert_eq!(
        a.provider_document("reference:reference-worker")
            .await
            .unwrap()
            .as_deref(),
        Some("reference-one")
    );
    assert_eq!(a.content_references(&row).await.unwrap().token, state.token);
    assert_eq!(a.content_references(&row).await.unwrap().blocked.len(), 2);
    a.promote_content_reference(&row, "reference-two", &state.token)
        .await
        .unwrap();
    assert_eq!(
        a.provider_document("reference:reference-worker")
            .await
            .unwrap()
            .as_deref(),
        Some("reference-two")
    );
    let next = a.content_references(&row).await.unwrap();
    assert_ne!(next.token, state.token);
    assert_eq!(next.blocked, ["reference-one"]);
    a.promote_content_reference(&row, "reference-one", &next.token)
        .await
        .unwrap();
    assert!(a
        .provider_document("content:references:reference-worker")
        .await
        .unwrap()
        .is_none());
}
async fn insert(repository: &AccountRepository, id: &str) -> Message {
    let mut row = message(&repository.account, id);
    row.received_at = "2090-01-01T00:00:00.000Z".into();
    repository
        .ingest_messages(vec![row], &[], &[], None)
        .await
        .unwrap()
        .remove(0)
}
async fn due(repository: &AccountRepository, id: &str) {
    match &repository.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE model_jobs SET retry_at=0 WHERE id=?")
                .bind(id)
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE model_jobs SET retry_at=0 WHERE account_id=$1 AND id=$2")
                .bind(&repository.account)
                .bind(id)
                .execute(db)
                .await
                .unwrap();
        }
    }
}
async fn models(a: &AccountRepository, b: &AccountRepository) {
    let original = insert(a, "model-message").await;
    insert(b, "model-message").await;
    assert_eq!(
        a.classification_candidate().await.unwrap().unwrap().id,
        original.id
    );
    let queued = a
        .queue_model(
            &original.id,
            JobKind::Classify,
            "fixture-model",
            Some("private fixture source"),
        )
        .await
        .unwrap();
    assert_eq!(queued.content_revision, original.revision);
    assert_eq!(
        a.provider_document(&format!("job-source:{}", queued.id))
            .await
            .unwrap()
            .as_deref(),
        Some("private fixture source")
    );
    assert!(matches!(
        b.model_job(&queued.id).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        b.defer_model_failure(&queued, ModelFailureReason::ContentUnavailable)
            .await,
        Err(Error::Invalid)
    ));
    assert!(a
        .classification_candidate()
        .await
        .unwrap()
        .is_none_or(|m| m.id != original.id));
    assert_eq!(
        b.classification_candidate().await.unwrap().unwrap().id,
        original.id
    );
    let mut claims = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let a = a.clone();
        claims.spawn(async move { a.claim_model().await.unwrap() });
    }
    let mut claimed = None;
    while let Some(result) = claims.join_next().await {
        if let Some(job) = result.unwrap() {
            assert!(claimed.replace(job).is_none());
        }
    }
    let mut job = claimed.unwrap();
    assert_eq!(job.id, queued.id);
    assert_eq!(job.status, JobStatus::Running);
    for retry in [true, true, false] {
        assert_eq!(
            a.defer_model_failure(&job, ModelFailureReason::ContentUnavailable)
                .await
                .unwrap(),
            retry
        );
        assert_eq!(
            a.model_failure_reason(&job.id).await.unwrap().as_deref(),
            Some("content_unavailable")
        );
        if retry {
            assert_eq!(
                a.model_job(&job.id).await.unwrap().status,
                JobStatus::Pending
            );
            assert!(a.claim_model().await.unwrap().is_none());
            due(a, &job.id).await;
            job = a.claim_model().await.unwrap().unwrap();
        }
    }
    job.status = JobStatus::Failed;
    a.finish_model(job, None, None).await.unwrap();
    assert_eq!(
        a.message(&original.id).await.unwrap().category_status,
        CategoryStatus::Failed
    );
    assert_eq!(
        b.message(&original.id).await.unwrap().category_status,
        CategoryStatus::Pending
    );

    let read = insert(a, "model-read").await;
    let queued = a
        .queue_model(&read.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    a.operation(mutation(
        &read,
        "model-read-operation",
        Change::Read { value: true },
    ))
    .await
    .unwrap();
    let mut running = a.claim_model().await.unwrap().unwrap();
    assert_eq!(running.id, queued.id);
    running.status = JobStatus::Succeeded;
    a.finish_model(
        running.clone(),
        Some(&crate::classification::legacy_classification("newsletters")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        a.model_job(&running.id).await.unwrap().status,
        JobStatus::Failed
    );
    assert_eq!(
        a.model_failure_reason(&running.id)
            .await
            .unwrap()
            .as_deref(),
        Some("revision_changed")
    );
    assert_eq!(
        a.message(&read.id).await.unwrap().category_status,
        CategoryStatus::Pending
    );
    a.queue_model(&read.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let mut running = a.claim_model().await.unwrap().unwrap();
    running.status = JobStatus::Succeeded;
    a.finish_model(
        running.clone(),
        Some(&crate::classification::legacy_classification("newsletters")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        a.model_job(&running.id).await.unwrap().status,
        JobStatus::Succeeded
    );
    assert_eq!(
        a.message(&read.id).await.unwrap().category.as_deref(),
        Some("notifications")
    );

    let manual = insert(a, "model-manual").await;
    a.queue_model(&manual.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let mut running = a.claim_model().await.unwrap().unwrap();
    a.operation(mutation(
        &manual,
        "manual-category",
        Change::Category {
            value: "people".into(),
        },
    ))
    .await
    .unwrap();
    running.status = JobStatus::Succeeded;
    a.finish_model(
        running.clone(),
        Some(&crate::classification::legacy_classification("newsletters")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        a.model_failure_reason(&running.id)
            .await
            .unwrap()
            .as_deref(),
        Some("revision_changed")
    );
    assert_eq!(
        a.message(&manual.id).await.unwrap().category.as_deref(),
        Some("people")
    );
    a.queue_model(&manual.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let mut current = a.claim_model().await.unwrap().unwrap();
    current.status = JobStatus::Succeeded;
    a.finish_model(
        current,
        Some(&crate::classification::legacy_classification("newsletters")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        a.message(&manual.id).await.unwrap().category.as_deref(),
        Some("people")
    );

    let expired = insert(a, "model-expired").await;
    let queued = a
        .queue_model(&expired.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE model_jobs SET document=json_set(document,'$.deadline','2000-01-01T00:00:00Z') WHERE id=?").bind(&queued.id).execute(db).await.unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE model_jobs SET document=jsonb_set(document::jsonb,'{deadline}','\"2000-01-01T00:00:00Z\"')::TEXT WHERE account_id=$1 AND id=$2").bind(&a.account).bind(&queued.id).execute(db).await.unwrap();
        }
    }
    let mut expired_job = a.claim_model().await.unwrap().unwrap();
    expired_job.status = JobStatus::Succeeded;
    a.finish_model(
        expired_job,
        Some(&crate::classification::legacy_classification("people")),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        a.model_failure_reason(&queued.id).await.unwrap().as_deref(),
        Some("deadline")
    );
    assert_eq!(
        a.message(&expired.id).await.unwrap().category_status,
        CategoryStatus::Pending
    );
    let row = insert(a, "model-expired-running").await;
    a.queue_model(&row.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let active = a.claim_model().await.unwrap().unwrap();
    assert!(a.claim_model().await.unwrap().is_none());
    assert_eq!(
        a.model_job(&active.id).await.unwrap().status,
        JobStatus::Running
    );
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE model_jobs SET document=json_set(document,'$.deadline','2000-01-01T00:00:00Z') WHERE id=?").bind(&active.id).execute(db).await.unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE model_jobs SET document=jsonb_set(document::jsonb,'{deadline}','\"2000-01-01T00:00:00Z\"')::TEXT WHERE account_id=$1 AND id=$2").bind(&a.account).bind(&active.id).execute(db).await.unwrap();
        }
    }
    assert!(a.claim_model().await.unwrap().is_none());
    assert_eq!(
        a.model_job(&active.id).await.unwrap().status,
        JobStatus::Failed
    );
    for (id, read) in [
        ("model-interrupted", false),
        ("model-interrupted-read", true),
    ] {
        let row = insert(a, id).await;
        a.queue_model(id, JobKind::Classify, "fixture", None)
            .await
            .unwrap();
        let job = a.claim_model().await.unwrap().unwrap();
        if read {
            a.operation(mutation(
                &row,
                &format!("read:{id}"),
                Change::Read { value: true },
            ))
            .await
            .unwrap();
        }
        a.recover_models().await.unwrap();
        assert_eq!(
            a.model_job(&job.id).await.unwrap().status,
            JobStatus::Failed
        );
        assert_eq!(
            a.model_failure_reason(&job.id).await.unwrap().as_deref(),
            Some("interrupted")
        );
    }
    let translated = insert(a, "model-translation").await;
    a.queue_model(&translated.id, JobKind::Translate, "fixture", None)
        .await
        .unwrap();
    let mut job = a.claim_model().await.unwrap().unwrap();
    job.status = JobStatus::Succeeded;
    let content:MessageContent=serde_json::from_value(serde_json::json!({"message":translated,"message_id":null,"references":[],"cc":[],"reply_to":[],"text":"Translated fixture","html":"","attachments":[],"detected_language":null})).unwrap();
    let mut foreign = content.clone();
    foreign.message.account_id = b.account.clone();
    assert!(matches!(
        a.finish_model(job.clone(), None, Some(("source", &foreign)))
            .await,
        Err(Error::Invalid)
    ));
    assert_eq!(
        a.model_job(&job.id).await.unwrap().status,
        JobStatus::Running
    );
    assert!(a
        .provider_document("translation:model-translation:source")
        .await
        .unwrap()
        .is_none());
    a.finish_model(job.clone(), None, Some(("source", &content)))
        .await
        .unwrap();
    let revision = a.message("model-translation").await.unwrap().revision;
    a.finish_model(job.clone(), None, Some(("source", &content)))
        .await
        .unwrap();
    assert_eq!(
        a.model_job(&job.id).await.unwrap().status,
        JobStatus::Succeeded
    );
    assert_eq!(
        a.message("model-translation").await.unwrap().revision,
        revision
    );
    assert!(a
        .provider_document("translation:model-translation:source")
        .await
        .unwrap()
        .is_some());
    assert!(b
        .provider_document("translation:model-translation:source")
        .await
        .unwrap()
        .is_none());
}

async fn arrival(
    repository: &AccountRepository,
    id: &str,
    category: Option<&str>,
    status: CategoryStatus,
) -> Message {
    let mut row = message(&repository.account, id);
    row.category = category.map(str::to_owned);
    row.category_status = status;
    let row = repository
        .ingest_messages(vec![row], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    repository
        .record_incremental(
            "INBOX",
            std::slice::from_ref(&row),
            "push-fixture",
            &[serde_json::json!({"id":"worker-push"})],
            &[],
        )
        .await
        .unwrap();
    row
}
async fn push_state(repository: &AccountRepository, message: &str) -> (i64, String) {
    match &repository.backend {
        Backend::Sqlite(db)=>sqlx::query_as("SELECT attempts,state FROM push_jobs WHERE message_id=? AND subscription_id='worker-push'").bind(message).fetch_one(db).await.unwrap(),
        Backend::Postgres(db)=>sqlx::query_as("SELECT attempts,state FROM push_jobs WHERE account_id=$1 AND message_id=$2 AND subscription_id='worker-push'").bind(&repository.account).bind(message).fetch_one(db).await.unwrap(),
    }
}
async fn notifications(a: &AccountRepository, b: &AccountRepository) {
    a.record_incremental("INBOX", &[], "push-bootstrap", &[], &[])
        .await
        .unwrap();
    let ready = arrival(a, "push-ready", Some("people"), CategoryStatus::Ready).await;
    let job = a.next_push(&[]).await.unwrap().unwrap();
    assert_eq!(job["message_id"], ready.id);
    assert!(a
        .push_send_allowed(&ready.id, "worker-push", &[])
        .await
        .unwrap());
    assert!(!b
        .push_send_allowed(&ready.id, "worker-push", &[])
        .await
        .unwrap());
    let ready = a.message(&ready.id).await.unwrap();
    a.operation(mutation(
        &ready,
        "push-newsletter",
        Change::Category {
            value: "newsletters".into(),
        },
    ))
    .await
    .unwrap();
    assert!(!a
        .push_send_allowed(&ready.id, "worker-push", &[])
        .await
        .unwrap());
    assert_eq!(push_state(a, &ready.id).await.1, "skipped");
    let waiting = arrival(a, "push-waiting", None, CategoryStatus::Pending).await;
    assert!(a.next_push(&[]).await.unwrap().is_none());
    assert_eq!(push_state(a, &waiting.id).await, (0, "pending".into()));
    a.queue_model(&waiting.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let mut job = a.claim_model().await.unwrap().unwrap();
    job.status = JobStatus::Failed;
    a.finish_model(job, None, None).await.unwrap();
    let first = a.next_push(&[]).await.unwrap().unwrap();
    assert_eq!(first["message_id"], waiting.id);
    for attempt in 1..=5 {
        assert_eq!(
            push_state(a, &waiting.id).await,
            (attempt, "running".into())
        );
        a.finish_push(&waiting.id, "worker-push", Some(0))
            .await
            .unwrap();
        let next = a.next_push(&[]).await.unwrap();
        if attempt < 5 {
            assert_eq!(next.unwrap(), first);
        } else {
            assert!(next.is_none());
        }
    }
    let fenced = arrival(a, "push-fenced", Some("people"), CategoryStatus::Ready).await;
    let (_, first_attempt) = a.next_push_claim(&[]).await.unwrap().unwrap();
    a.finish_push_attempt(&fenced.id, "worker-push", first_attempt, Some(0))
        .await
        .unwrap();
    let (_, second_attempt) = a.next_push_claim(&[]).await.unwrap().unwrap();
    assert_eq!(second_attempt, first_attempt + 1);
    // The first completion committed but its acknowledgement was lost.
    a.finish_push_attempt(&fenced.id, "worker-push", first_attempt, Some(0))
        .await
        .unwrap();
    assert_eq!(
        push_state(a, &fenced.id).await,
        (second_attempt, "running".into())
    );
    assert!(a.next_push_claim(&[]).await.unwrap().is_none());
    a.finish_push_attempt(&fenced.id, "worker-push", second_attempt, None)
        .await
        .unwrap();
    let row = arrival(a, "push-recheck", Some("people"), CategoryStatus::Ready).await;
    a.next_push(&[]).await.unwrap().unwrap();
    // Simulate classification becoming pending after the push was claimed.
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE messages SET document=json_set(document,'$.classification.status','pending','$.category_status','pending') WHERE id=?")
                .bind(&row.id).execute(db).await.unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE messages SET document=jsonb_set(jsonb_set(document::jsonb,'{classification,status}','\"pending\"'),'{category_status}','\"pending\"')::TEXT WHERE account_id=$1 AND id=$2")
                .bind(&a.account).bind(&row.id).execute(db).await.unwrap();
        }
    }
    assert!(!a
        .push_send_allowed(&row.id, "worker-push", &[])
        .await
        .unwrap());
    assert_eq!(push_state(a, &row.id).await, (0, "pending".into()));
    a.queue_model(&row.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let mut job = a.claim_model().await.unwrap().unwrap();
    job.status = JobStatus::Succeeded;
    a.finish_model(
        job,
        Some(&crate::classification::legacy_classification("people")),
        None,
    )
    .await
    .unwrap();
    let before = a.next_push(&[]).await.unwrap().unwrap();
    a.recover_push(&[]).await.unwrap();
    let after = a.next_push(&[]).await.unwrap().unwrap();
    assert_eq!(before, after);
    a.finish_push(&row.id, "worker-push", None).await.unwrap();
    assert_eq!(push_state(a, &row.id).await.1, "done");
    let mut blocked = message(&a.account, "push-blocked");
    blocked.sender.email = "BÖB@EXAMPLE.INVALID".into();
    blocked.category_status = CategoryStatus::Ready;
    let blocked = a
        .ingest_messages(vec![blocked], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    a.record_incremental(
        "INBOX",
        std::slice::from_ref(&blocked),
        "push-blocked",
        &[serde_json::json!({"id":"worker-push"})],
        &[],
    )
    .await
    .unwrap();
    let rule:MailRule=serde_json::from_value(serde_json::json!({"id":"worker-block","preset":"block_sender","enabled":true,"activated_at":"2026-01-01T00:00:00Z","scope":"all_mailboxes","condition":{"kind":"sender_is","email":"böb@example.invalid"},"actions":["trash","suppress_notifications"]})).unwrap();
    assert!(a.next_push(&[rule]).await.unwrap().is_none());
    assert_eq!(push_state(a, &blocked.id).await.1, "skipped");
    let snoozed = insert(a, "worker-snooze").await;
    a.operation(mutation(
        &snoozed,
        "worker-snooze-operation",
        Change::Snooze {
            until: "2099-01-01T00:00:00Z".into(),
        },
    ))
    .await
    .unwrap();
    assert!(a.is_snoozed(&snoozed.thread_id).await.unwrap());
    assert!(!b.is_snoozed(&snoozed.thread_id).await.unwrap());
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE snoozes SET until='2000-01-01T00:00:00Z' WHERE thread_id=?")
                .bind(&snoozed.thread_id)
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE snoozes SET until='2000-01-01T00:00:00Z' WHERE account_id=$1 AND thread_id=$2").bind(&a.account).bind(&snoozed.thread_id).execute(db).await.unwrap();
        }
    }
    let cursor = a.event_cursor().await.unwrap();
    a.wake_snoozes().await.unwrap();
    assert!(!a.is_snoozed(&snoozed.thread_id).await.unwrap());
    assert_eq!(a.events(cursor).await.unwrap()[0].kind, "snooze_woke");
}
