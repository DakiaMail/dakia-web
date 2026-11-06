use super::*;

#[tokio::test]
async fn newsletter_push_waits_for_classification_and_is_terminal_after_restart() {
    let (dir, store, user, a, _) = fixture().await;
    for id in ["one", "two"] {
        store
            .subscribe(
                &user,
                id,
                &serde_json::json!({"id":id,"user_id":user}).to_string(),
            )
            .await
            .unwrap();
    }
    store
        .record_incremental(&a, "INBOX", &[], "bootstrap")
        .await
        .unwrap();
    let mut arrival = message(&a, "newsletter");
    arrival.received_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let arrival = store.upsert_message(&a, arrival).await.unwrap();
    store
        .record_incremental(&a, "INBOX", &[arrival], "arrival")
        .await
        .unwrap();
    assert!(
        store.next_push(&a).await.unwrap().is_none(),
        "pending classification must not reach transport"
    );
    let mut job = store
        .queue_model(&a, "newsletter", JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    job.status = JobStatus::Succeeded;
    store
        .finish_model(&a, job, Some("newsletters"), None)
        .await
        .unwrap();
    assert!(store.next_push(&a).await.unwrap().is_none());
    let db = store.db(&a).await.unwrap();
    let states: Vec<String> =
        sqlx::query_scalar("SELECT state FROM push_jobs ORDER BY subscription_id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(states, vec!["skipped", "skipped"]);
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = reopened.authorize(&user, a.id()).await.unwrap();
    reopened.recover_push(&a).await.unwrap();
    assert!(reopened.next_push(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn newsletter_is_skipped_at_admission_without_consuming_attempts() {
    let (_dir, store, user, a, _) = fixture().await;
    store
        .subscribe(&user, "one", "{\"id\":\"one\"}")
        .await
        .unwrap();
    store
        .record_incremental(&a, "INBOX", &[], "bootstrap")
        .await
        .unwrap();
    let mut m = message(&a, "classified");
    m.received_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    m.category = Some("newsletters".into());
    m.category_status = CategoryStatus::Ready;
    let m = store.upsert_message(&a, m).await.unwrap();
    for _ in 0..2 {
        store
            .record_incremental(&a, "INBOX", std::slice::from_ref(&m), "arrival")
            .await
            .unwrap();
    }
    let db = store.db(&a).await.unwrap();
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT state,attempts FROM push_jobs")
        .fetch_all(&db)
        .await
        .unwrap();
    assert_eq!(rows, vec![("skipped".into(), 0)]);
}

#[tokio::test]
async fn delayed_classification_preserves_a_valid_delivery_deadline() {
    let (_dir, store, user, a, _) = fixture().await;
    store
        .subscribe(&user, "one", "{\"id\":\"one\"}")
        .await
        .unwrap();
    store
        .record_incremental(&a, "INBOX", &[], "bootstrap")
        .await
        .unwrap();
    let mut m = message(&a, "delayed");
    m.received_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let m = store.upsert_message(&a, m).await.unwrap();
    store
        .record_incremental(&a, "INBOX", &[m], "arrival")
        .await
        .unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query(
        "UPDATE push_jobs SET document=json_set(document,'$.deadline','2000-01-01T00:00:00Z')",
    )
    .execute(&db)
    .await
    .unwrap();
    assert!(store.next_push(&a).await.unwrap().is_none());
    let mut job = store
        .queue_model(&a, "delayed", JobKind::Classify, "fixture", None)
        .await
        .unwrap();
    job.status = JobStatus::Succeeded;
    store
        .finish_model(&a, job, Some("transactions"), None)
        .await
        .unwrap();
    let job = store.next_push(&a).await.unwrap().unwrap();
    assert!(
        DateTime::parse_from_rfc3339(job["deadline"].as_str().unwrap())
            .unwrap()
            .timestamp()
            > Utc::now().timestamp()
    );
}

#[tokio::test]
async fn final_push_decision_rechecks_category_and_never_revives_skipped_retries() {
    let (_dir, store, user, a, _) = fixture().await;
    store
        .subscribe(&user, "one", "{\"id\":\"one\"}")
        .await
        .unwrap();
    store
        .record_incremental(&a, "INBOX", &[], "bootstrap")
        .await
        .unwrap();
    let db = store.db(&a).await.unwrap();
    for id in [
        "reclassified",
        "retry",
        "recovered",
        "waiting",
        "failed",
        "unknown",
        "transaction",
        "manual",
    ] {
        let mut m = message(&a, id);
        m.received_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
        m.category = Some("people".into());
        m.category_status = CategoryStatus::Ready;
        let m = store.upsert_message(&a, m).await.unwrap();
        store
            .record_incremental(&a, "INBOX", &[m], id)
            .await
            .unwrap();
        assert!(store.next_push(&a).await.unwrap().is_some());
        match id {
            "reclassified" | "retry" | "recovered" => {
                if id == "retry" {
                    store.finish_push(&a, id, "one", Some(0)).await.unwrap();
                }
                sqlx::query("UPDATE messages SET document=json_set(document,'$.category','newsletters') WHERE id=?").bind(id).execute(&db).await.unwrap();
                if id == "recovered" {
                    store.recover_push(&a).await.unwrap();
                }
                assert!(!store.push_send_allowed(&a, id, "one").await.unwrap());
                // A late completion from an earlier attempt cannot resurrect it.
                store.finish_push(&a, id, "one", Some(0)).await.unwrap();
                store.recover_push(&a).await.unwrap();
                assert!(store.next_push(&a).await.unwrap().is_none());
                let state: String =
                    sqlx::query_scalar("SELECT state FROM push_jobs WHERE message_id=?")
                        .bind(id)
                        .fetch_one(&db)
                        .await
                        .unwrap();
                assert_eq!(state, "skipped");
            }
            "waiting" => {
                sqlx::query("UPDATE messages SET document=json_set(document,'$.category',NULL,'$.category_status','pending') WHERE id=?").bind(id).execute(&db).await.unwrap();
                assert!(!store.push_send_allowed(&a, id, "one").await.unwrap());
                assert!(store.next_push(&a).await.unwrap().is_none());
            }
            _ => {
                let (category, status) = match id {
                    "failed" => (None, "failed"),
                    "unknown" => (None, "ready"),
                    "transaction" => (Some("transactions"), "ready"),
                    _ => (Some("newsletters"), "pending"),
                };
                sqlx::query("UPDATE messages SET document=json_set(document,'$.category',?,'$.category_status',?),category_override=? WHERE id=?")
                    .bind(category).bind(status).bind(if id == "manual" {Some("people")} else {None}).bind(id).execute(&db).await.unwrap();
                assert!(store.push_send_allowed(&a, id, "one").await.unwrap());
                assert!(!store
                    .push_send_allowed(&a, id, "foreign-subscription")
                    .await
                    .unwrap());
                store.finish_push(&a, id, "one", None).await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn bootstrap_arrivals_replay_restart_folders_and_accounts_preserve_push_dedup() {
    let (dir, store, user, a, input) = fixture().await;
    let subscription = serde_json::json!({"id":"disposable","user_id":user,"endpoint":"https://fcm.googleapis.com/disposable","keys":{"p256dh":"fixture","auth":"fixture"}});
    store
        .subscribe(&user, "disposable", &subscription.to_string())
        .await
        .unwrap();
    let mut historical = message(&a, "history");
    historical.received_at = (Utc::now() + Duration::days(1)).to_rfc3339();
    let historical = store.upsert_message(&a, historical).await.unwrap();
    store
        .record_incremental(&a, "INBOX", &[historical], "bootstrap")
        .await
        .unwrap();
    assert!(store.next_push(&a).await.unwrap().is_none());
    let mut arrival = message(&a, "arrival");
    arrival.category = Some("people".into());
    arrival.category_status = CategoryStatus::Ready;
    arrival.received_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let arrival = store.upsert_message(&a, arrival).await.unwrap();
    // Archive/label discovery is not itself new INBOX mail.
    store
        .record_incremental(&a, "Archive", std::slice::from_ref(&arrival), "archive")
        .await
        .unwrap();
    assert!(store.next_push(&a).await.unwrap().is_none());
    for _ in 0..3 {
        store
            .record_incremental(&a, "INBOX", std::slice::from_ref(&arrival), "arrival")
            .await
            .unwrap();
    }
    let first = store.next_push(&a).await.unwrap().unwrap();
    assert!(store.next_push(&a).await.unwrap().is_none());
    // Crash after claim, possibly after HTTP delivery: recovered job retains its
    // exact ID (and therefore browser notification tag), not a fresh alert.
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = reopened.authorize(&user, a.id()).await.unwrap();
    reopened.recover_push(&a).await.unwrap();
    let replay = reopened.next_push(&a).await.unwrap().unwrap();
    assert_eq!(first, replay);
    reopened
        .finish_push(&a, "arrival", "disposable", None)
        .await
        .unwrap();
    reopened
        .record_incremental(&a, "INBOX", std::slice::from_ref(&arrival), "replay")
        .await
        .unwrap();
    reopened.recover_push(&a).await.unwrap();
    assert!(reopened.next_push(&a).await.unwrap().is_none());
    let other = reopened.connect(&user, &input).await.unwrap();
    let b = reopened.authorize(&user, &other.id).await.unwrap();
    reopened
        .record_incremental(&b, "INBOX", &[], "bootstrap-b")
        .await
        .unwrap();
    let mut twin = arrival;
    twin.account_id = b.id().into();
    let twin = reopened.upsert_message(&b, twin).await.unwrap();
    reopened
        .record_incremental(&b, "INBOX", &[twin], "b-arrival")
        .await
        .unwrap();
    let other = reopened.next_push(&b).await.unwrap().unwrap();
    assert_ne!(other["account_id"], first["account_id"]);
    assert_ne!(other["id"], first["id"]);
    assert!(reopened.next_push(&a).await.unwrap().is_none());
}
