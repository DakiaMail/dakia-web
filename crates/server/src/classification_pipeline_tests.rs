use super::*;

#[tokio::test]
async fn completed_classification_retries_only_persistence_after_a_transient_failure() {
    let (dir, app, _, a, _) = controlled().await;
    let message = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows
        .remove(0);
    app.store
        .queue_model(&a, &message.id, JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    let mut job = app.store.claim_model(&a).await.unwrap().unwrap();
    job.status = JobStatus::Succeeded;
    let db = database(&dir, &a).await;
    sqlx::query("CREATE TRIGGER fail_completed_model BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT,'synthetic persistence failure'); END").execute(&db).await.unwrap();
    let repair = {
        let db = db.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            sqlx::query("DROP TRIGGER fail_completed_model")
                .execute(&db)
                .await
                .unwrap();
        })
    };
    let classification = Classification {
        status: CategoryStatus::Ready,
        ..Default::default()
    };
    crate::models::persist_outcome(&app.store, &a, &job, Some(&classification), None)
        .await
        .unwrap();
    repair.await.unwrap();
    assert_eq!(
        app.store.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Succeeded
    );
    let revision = app.store.message(&a, &message.id).await.unwrap().revision;
    let cursor = app.store.event_cursor(&a).await.unwrap();
    crate::models::persist_outcome(&app.store, &a, &job, Some(&classification), None)
        .await
        .unwrap();
    assert_eq!(
        app.store.message(&a, &message.id).await.unwrap().revision,
        revision
    );
    assert_eq!(app.store.event_cursor(&a).await.unwrap(), cursor);
    db.close().await;
}

#[tokio::test]
async fn cached_read_backfill_keeps_moving_and_separates_notices_from_correspondence() {
    let (_dir, app, _, a, provider) = controlled().await;
    app.store
        .account_state(&a, AccountState::Ready)
        .await
        .unwrap();
    let rows = app
        .mail
        .snapshot_page(&app.store, &a, "INBOX", None)
        .await
        .unwrap()
        .rows;
    let mut ids = Vec::new();
    for (i, mut m) in rows.into_iter().enumerate() {
        m.unread = false;
        m.classification = Some(Classification::default());
        m.subject = if i == 0 {
            "Your Example receipt [#12345]"
        } else {
            "Re: Billing: Refund Request"
        }
        .into();
        m = app.store.upsert_message(&a, m).await.unwrap();
        app.store
            .learn_recipients(
                &a,
                "draft:classification-fixture",
                &[m.sender.clone()],
                &m.received_at,
            )
            .await
            .unwrap();
        let body = if i == 0 {
            "Total: EUR 42.00"
        } else {
            "Hi, I can help with your refund."
        };
        let raw = format!(
            "From: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}",
            m.sender.email, m.subject
        );
        let content: MessageContent = serde_json::from_value(serde_json::json!({"message":m,"text":body,"html":"","attachments":[],"message_id":null,"references":[],"cc":[],"reply_to":[]})).unwrap();
        app.store
            .cache_content_with_raw(&a, &content, Some(raw.as_bytes()), 1024 * 1024)
            .await
            .unwrap();
        ids.push(m.id.clone());
    }
    app.store
        .requeue_classification_policy(&a, dakia_inference::POLICY_REVISION)
        .await
        .unwrap();
    let mut protected = app.store.message(&a, &ids[0]).await.unwrap();
    protected.id = "protected-claim".into();
    protected.provider_id = "protected-claim".into();
    let protected = app.store.upsert_message(&a, protected).await.unwrap();
    app.store
        .queue_model(
            &a,
            &protected.id,
            JobKind::Classify,
            dakia_inference::POLICY_REVISION,
            None,
        )
        .await
        .unwrap();
    let mut owned = app.store.claim_model(&a).await.unwrap().unwrap();
    let worker = tokio::spawn(crate::models::run(app.clone()));
    let completed = tokio::time::timeout(Duration::from_millis(1500), async {
        loop {
            let mut ready = true;
            for id in &ids {
                ready &= app
                    .store
                    .message(&a, id)
                    .await
                    .unwrap()
                    .classification
                    .unwrap()
                    .status
                    == CategoryStatus::Ready;
            }
            if ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    worker.abort();
    completed.expect("cached backfill must not idle for two seconds after each message");
    assert_eq!(
        app.store.model_job(&a, &owned.id).await.unwrap().status,
        JobStatus::Running
    );
    owned.status = JobStatus::Succeeded;
    crate::models::persist_outcome(
        &app.store,
        &a,
        &owned,
        Some(&Classification {
            status: CategoryStatus::Ready,
            ..Classification::default()
        }),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        app.store.model_job(&a, &owned.id).await.unwrap().status,
        JobStatus::Succeeded
    );

    assert_eq!(
        app.store.message(&a, &ids[0]).await.unwrap().inbox_kind(),
        InboxKind::Automated
    );
    assert_eq!(
        app.store.message(&a, &ids[1]).await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
    for id in ids {
        assert!(!app.store.message(&a, &id).await.unwrap().unread);
    }
    assert_eq!(provider.fetch_calls.load(Ordering::SeqCst), 0);
}
