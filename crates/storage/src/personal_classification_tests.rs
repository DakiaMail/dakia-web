use super::*;
async fn change(store: &Store, a: &AuthorizedAccount, id: &str, change: Change) {
    let m = store.message(a, id).await.unwrap();
    let result = store
        .operation(
            a,
            Operation {
                id: uuid::Uuid::new_v4().to_string(),
                account_id: a.id().into(),
                target_id: id.into(),
                expected_revision: m.revision,
                change,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    assert!(
        result.status == OperationStatus::Applied || matches!(result.change, Change::Read { .. })
    );
}
async fn detect(store: &Store, a: &AuthorizedAccount, id: &str, c: Classification) {
    store
        .queue_model(a, id, JobKind::Classify, "personal-first-1", None)
        .await
        .unwrap();
    let mut job = store.claim_model(a).await.unwrap().unwrap();
    job.status = JobStatus::Succeeded;
    store
        .finish_classification(a, job, Some(&c), None)
        .await
        .unwrap();
}
fn automated(types: Vec<DetectedType>) -> Classification {
    Classification {
        kind: InboxKind::Automated,
        status: CategoryStatus::Ready,
        types,
        policy_revision: "personal-first-1".into(),
        ..Default::default()
    }
}
#[tokio::test]
async fn read_mail_is_classified_and_overlapping_filters_run_before_paging() {
    let (_d, s, _u, a, _) = fixture().await;
    for i in 0..62 {
        let mut m = message(&a, &format!("m{i:03}"));
        m.unread = false;
        s.upsert_message(&a, m).await.unwrap();
        detect(
            &s,
            &a,
            &format!("m{i:03}"),
            automated(if i == 0 {
                vec![DetectedType::Receipt, DetectedType::Order]
            } else {
                vec![]
            }),
        )
        .await;
    }
    assert!(s.classification_candidate(&a).await.unwrap().is_none());
    let filter = ClassificationFilter {
        inbox_kind: Some(InboxKind::Automated),
        email_type: Some(DetectedType::Receipt),
        attention: false,
        transactions: false,
    };
    let rows = s
        .inbox_rows_filtered(&a, None, None, &filter)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "m000");
    let filter = CachedSearchFilter {
        classification: filter,
        ..Default::default()
    };
    assert_eq!(s.cached_search_rows(&a, &filter).await.unwrap().len(), 1);
    s.index_search_headers(&a).await.unwrap();
    assert_eq!(s.full_text_rows(&a, &filter, None).await.unwrap().len(), 1);
}
#[tokio::test]
async fn corrections_sender_precedence_types_attention_and_restart() {
    let (d, s, _u, a, _) = fixture().await;
    for id in ["one", "two", "three"] {
        s.upsert_message(&a, message(&a, id)).await.unwrap();
        detect(&s, &a, id, automated(vec![DetectedType::Order])).await;
    }
    change(
        &s,
        &a,
        "two",
        Change::InboxKind {
            value: Some(InboxKind::Automated),
            sender: false,
        },
    )
    .await;
    change(
        &s,
        &a,
        "one",
        Change::InboxKind {
            value: Some(InboxKind::Personal),
            sender: true,
        },
    )
    .await;
    assert_eq!(
        s.message(&a, "one").await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
    assert_eq!(
        s.message(&a, "two").await.unwrap().inbox_kind(),
        InboxKind::Automated
    );
    assert_eq!(
        s.message(&a, "three").await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
    change(
        &s,
        &a,
        "one",
        Change::DetectedType {
            value: DetectedType::Order,
            present: false,
        },
    )
    .await;
    change(
        &s,
        &a,
        "one",
        Change::DetectedType {
            value: DetectedType::FlightBooking,
            present: true,
        },
    )
    .await;
    change(&s, &a, "one", Change::Attention { value: Some(false) }).await;
    let mut c = automated(vec![DetectedType::Order]);
    c.attention.push(AttentionReason {
        label: "Flight changed".into(),
        expires_at: None,
    });
    detect(&s, &a, "one", c).await;
    let reopened = Store::open(d.path(), [4; 32]).await.unwrap();
    let auth = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    let m = reopened.message(&auth, "one").await.unwrap();
    let c = m.classification.unwrap();
    assert_eq!(c.kind, InboxKind::Personal);
    assert_eq!(c.types, vec![DetectedType::FlightBooking]);
    assert!(c.details.is_empty());
    assert!(!c.needs_attention("2026-11-12"));
    change(
        &reopened,
        &auth,
        "one",
        Change::InboxKind {
            value: None,
            sender: true,
        },
    )
    .await;
    assert_eq!(
        reopened.message(&auth, "three").await.unwrap().inbox_kind(),
        InboxKind::Automated
    );
    change(
        &reopened,
        &auth,
        "two",
        Change::InboxKind {
            value: Some(InboxKind::Personal),
            sender: false,
        },
    )
    .await;
    change(
        &reopened,
        &auth,
        "one",
        Change::InboxKind {
            value: Some(InboxKind::Automated),
            sender: true,
        },
    )
    .await;
    change(
        &reopened,
        &auth,
        "two",
        Change::InboxKind {
            value: None,
            sender: true,
        },
    )
    .await;
    assert_eq!(
        reopened
            .message(&auth, "two")
            .await
            .unwrap()
            .classification
            .unwrap()
            .source,
        ClassificationSource::Message
    );
    assert_eq!(
        reopened.message(&auth, "two").await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
    // Sender preferences are scoped to one authorized account.
    let user = s.user("issuer", "owner").await.unwrap();
    let input=serde_json::from_value(serde_json::json!({"email":"other@example.invalid","display_name":"Other","provider":"imap","imap_host":"imap.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.invalid","smtp_port":465,"smtp_tls":"implicit","username":"other","password":"fixture-only"})).unwrap();
    let b = s.connect(&user, &input).await.unwrap();
    let b = s.authorize(&user, &b.id).await.unwrap();
    s.upsert_message(&b, message(&b, "one")).await.unwrap();
    assert_eq!(
        s.message(&b, "one").await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
}
#[tokio::test]
async fn attention_expiry_and_manual_dismissal_are_independent_of_kind_and_read_state() {
    let (_d, s, _u, a, _) = fixture().await;
    s.upsert_message(&a, message(&a, "code")).await.unwrap();
    let mut c = automated(vec![DetectedType::VerificationCode]);
    c.attention.push(AttentionReason {
        label: "Code".into(),
        expires_at: Some("2000-01-01T00:00:00.000Z".into()),
    });
    detect(&s, &a, "code", c).await;
    let filter = ClassificationFilter {
        attention: true,
        ..Default::default()
    };
    assert!(s
        .inbox_rows_filtered(&a, None, None, &filter)
        .await
        .unwrap()
        .is_empty());
    change(&s, &a, "code", Change::Attention { value: Some(true) }).await;
    change(&s, &a, "code", Change::Read { value: true }).await;
    assert_eq!(
        s.inbox_rows_filtered(&a, None, None, &filter)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.message(&a, "code").await.unwrap().inbox_kind(),
        InboxKind::Automated
    );
    change(&s, &a, "code", Change::Attention { value: Some(false) }).await;
    assert!(s
        .inbox_rows_filtered(&a, None, None, &filter)
        .await
        .unwrap()
        .is_empty());
}
#[tokio::test]
async fn stale_jobs_cannot_overwrite_corrections_and_failed_detection_stays_personal() {
    let (_d, s, _u, a, _) = fixture().await;
    s.upsert_message(&a, message(&a, "one")).await.unwrap();
    s.queue_model(&a, "one", JobKind::Classify, "personal-first-1", None)
        .await
        .unwrap();
    let mut job = s.claim_model(&a).await.unwrap().unwrap();
    change(
        &s,
        &a,
        "one",
        Change::InboxKind {
            value: Some(InboxKind::Personal),
            sender: false,
        },
    )
    .await;
    job.status = JobStatus::Succeeded;
    s.finish_classification(&a, job.clone(), Some(&automated(vec![])), None)
        .await
        .unwrap();
    assert_eq!(
        s.model_job(&a, &job.id).await.unwrap().status,
        JobStatus::Failed
    );
    assert_eq!(
        s.message(&a, "one").await.unwrap().inbox_kind(),
        InboxKind::Personal
    );
}

#[tokio::test]
async fn migration_preserves_legacy_choices_and_restorable_snapshot() {
    use sqlx::sqlite::SqliteConnectOptions;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.sqlite");
    let db = sqlx::SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    let mut old = sqlx::migrate!("./migrations/account");
    old.migrations.to_mut().retain(|m| m.version <= 20);
    old.run(&db).await.unwrap();
    for (id, category) in [
        ("personal", "people"),
        ("unknown", "other"),
        ("automatic", "transactions"),
        ("newsletter", "newsletters"),
    ] {
        let document = serde_json::json!({"id":id,"category":category,"category_status":"ready","received_at":"2026-11-12T10:00:00.000Z","folder_ids":["INBOX"],"unread":false});
        sqlx::query("INSERT INTO messages(id,revision,document,category_override) VALUES(?,1,?,?)")
            .bind(id)
            .bind(document.to_string())
            .bind(if id == "automatic" {
                None
            } else {
                Some(category)
            })
            .execute(&db)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO push_jobs(message_id,subscription_id,document,state) VALUES('personal','fixture','{}','pending')").execute(&db).await.unwrap();
    let backup = dir.path().join("backup.sqlite");
    sqlx::query("VACUUM INTO ?")
        .bind(backup.to_str().unwrap())
        .execute(&db)
        .await
        .unwrap();
    sqlx::migrate!("./migrations/account")
        .run(&db)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM push_jobs WHERE message_id='personal'")
            .fetch_one(&db)
            .await
            .unwrap(),
        "skipped"
    );
    let choices:Vec<(String,String,String)>=sqlx::query_as("SELECT id,json_extract(document,'$.classification.kind'),COALESCE(category_override,'') FROM messages ORDER BY id").fetch_all(&db).await.unwrap();
    assert_eq!(
        choices,
        vec![
            ("automatic".into(), "personal".into(), "".into()),
            (
                "newsletter".into(),
                "automated".into(),
                "newsletters".into()
            ),
            ("personal".into(), "personal".into(), "people".into()),
            ("unknown".into(), "personal".into(), "other".into())
        ]
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM messages WHERE json_array_length(document,'$.classification.types')=0 AND json_extract(document,'$.classification.status')='pending'").fetch_one(&db).await.unwrap(),4);
    let restored = sqlx::SqlitePool::connect_with(SqliteConnectOptions::new().filename(backup))
        .await
        .unwrap();
    sqlx::migrate!("./migrations/account")
        .run(&restored)
        .await
        .unwrap();
    let restored_choices:Vec<(String,String,String)>=sqlx::query_as("SELECT id,json_extract(document,'$.classification.kind'),COALESCE(category_override,'') FROM messages ORDER BY id").fetch_all(&restored).await.unwrap();
    assert_eq!(restored_choices, choices);
}

#[tokio::test]
async fn deleting_mail_removes_detected_facts_and_corrections() {
    let (_d, s, _u, a, _) = fixture().await;
    s.upsert_message(&a, message(&a, "deleted")).await.unwrap();
    detect(&s, &a, "deleted", automated(vec![DetectedType::Order])).await;
    let db = s.db(&a).await.unwrap();
    sqlx::query("DELETE FROM messages WHERE id='deleted'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM message_classifications WHERE message_id='deleted'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn policy_changes_requeue_read_and_manual_mail_once_without_losing_corrections() {
    let (_d, s, _u, a, _) = fixture().await;
    let mut m = message(&a, "old");
    m.unread = false;
    s.upsert_message(&a, m).await.unwrap();
    detect(&s, &a, "old", automated(vec![DetectedType::Order])).await;
    change(
        &s,
        &a,
        "old",
        Change::InboxKind {
            value: Some(InboxKind::Personal),
            sender: false,
        },
    )
    .await;
    change(
        &s,
        &a,
        "old",
        Change::DetectedType {
            value: DetectedType::FlightBooking,
            present: true,
        },
    )
    .await;
    s.requeue_classification_policy(&a, "next-policy")
        .await
        .unwrap();
    assert_eq!(
        s.classification_candidate(&a).await.unwrap().unwrap().id,
        "old"
    );
    let m = s.message(&a, "old").await.unwrap();
    assert_eq!(m.inbox_kind(), InboxKind::Personal);
    assert!(m
        .classification
        .unwrap()
        .types
        .contains(&DetectedType::FlightBooking));
    let revision = s.message(&a, "old").await.unwrap().revision;
    s.requeue_classification_policy(&a, "next-policy")
        .await
        .unwrap();
    assert_eq!(s.message(&a, "old").await.unwrap().revision, revision);
}

#[tokio::test]
async fn attention_age_limit_precedes_pagination_and_manual_marks() {
    let (_d, s, _u, a, _) = fixture().await;
    let now = Utc::now();
    for (id, age) in [("recent", 29), ("old", 31), ("future", -1)] {
        let mut m = message(&a, id);
        m.received_at = (now - Duration::days(age)).to_rfc3339();
        s.upsert_message(&a, m).await.unwrap();
        let mut c = automated(vec![DetectedType::EventInvitation]);
        c.attention.push(AttentionReason {
            label: "Event cancelled".into(),
            expires_at: None,
        });
        detect(&s, &a, id, c).await;
        change(&s, &a, id, Change::Attention { value: Some(true) }).await;
    }
    let filter = ClassificationFilter {
        attention: true,
        ..Default::default()
    };
    let rows = s
        .inbox_rows_filtered(&a, None, None, &filter)
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["recent"]
    );
    let search = CachedSearchFilter {
        classification: filter.clone(),
        ..Default::default()
    };
    assert_eq!(s.cached_search_rows(&a, &search).await.unwrap().len(), 1);
    s.index_search_headers(&a).await.unwrap();
    assert_eq!(s.full_text_rows(&a, &search, None).await.unwrap().len(), 1);
    let mut m = rows[0].clone();
    let now = "2026-11-13T12:00:00.000Z";
    for (received, expected) in [
        ("2026-10-14T12:00:00.000Z", true),
        ("2026-10-14T11:59:59.999Z", false),
        ("2026-11-13T12:00:00.001Z", false),
        ("2026-10-14T14:00:00+02:00", true),
        ("invalid", false),
    ] {
        m.received_at = received.into();
        assert_eq!(filter.matches(&m, now), expected, "{received}");
    }
}

#[tokio::test]
async fn attention_push_rechecks_age_at_delivery() {
    let (_d, s, user, a, _) = fixture().await;
    s.subscribe(&user, "one", "{\"id\":\"one\"}").await.unwrap();
    s.record_incremental(&a, "INBOX", &[], "bootstrap")
        .await
        .unwrap();
    let mut m = message(&a, "attention-arrival");
    m.received_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
    let m = s.upsert_message(&a, m).await.unwrap();
    s.record_incremental(&a, "INBOX", &[m], "arrival")
        .await
        .unwrap();
    let mut c = automated(vec![DetectedType::SecurityAlert]);
    c.attention.push(AttentionReason {
        label: "Account security".into(),
        expires_at: None,
    });
    detect(&s, &a, "attention-arrival", c).await;
    change(
        &s,
        &a,
        "attention-arrival",
        Change::Attention { value: Some(true) },
    )
    .await;
    let mut current = s.message(&a, "attention-arrival").await.unwrap();
    current.received_at = (Utc::now() - Duration::seconds(1)).to_rfc3339();
    s.upsert_message(&a, current).await.unwrap();
    assert!(s.next_push(&a).await.unwrap().is_some());
    let mut m = s.message(&a, "attention-arrival").await.unwrap();
    m.received_at = (Utc::now() - Duration::days(31)).to_rfc3339();
    s.upsert_message(&a, m).await.unwrap();
    assert!(!s
        .push_send_allowed(&a, "attention-arrival", "one")
        .await
        .unwrap());
    assert!(s.next_push(&a).await.unwrap().is_none());
}

#[tokio::test]
async fn transactions_combine_receipts_and_invoices_before_paging_without_duplicates() {
    let (_d, s, _u, a, _) = fixture().await;
    for i in 0..64 {
        let id = format!("transaction-{i:03}");
        s.upsert_message(&a, message(&a, &id)).await.unwrap();
        let types = if i == 0 {
            vec![DetectedType::Receipt, DetectedType::Invoice]
        } else if i == 63 {
            vec![DetectedType::Order]
        } else if i % 2 == 0 {
            vec![DetectedType::Receipt]
        } else {
            vec![DetectedType::Invoice]
        };
        detect(&s, &a, &id, automated(types)).await;
    }
    let filter = ClassificationFilter {
        transactions: true,
        ..Default::default()
    };
    let first = s
        .inbox_rows_filtered(&a, None, None, &filter)
        .await
        .unwrap();
    assert_eq!(first.len(), 51);
    let last = &first[49];
    let second = s
        .inbox_rows_filtered(&a, Some((&last.received_at, &last.id)), None, &filter)
        .await
        .unwrap();
    let ids = first
        .iter()
        .take(50)
        .chain(second.iter())
        .map(|m| m.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 63);
    assert!(!ids.contains("transaction-063"));
    assert_eq!(
        ids.iter()
            .filter(|id| id.as_str() == "transaction-000")
            .count(),
        1
    );
    for row in first.iter().chain(second.iter()) {
        assert!(filter.matches(row, &Utc::now().to_rfc3339()));
    }
    let search = CachedSearchFilter {
        classification: filter,
        ..Default::default()
    };
    assert_eq!(s.cached_search_rows(&a, &search).await.unwrap().len(), 50);
    s.index_search_headers(&a).await.unwrap();
    let indexed = s.full_text_rows(&a, &search, None).await.unwrap();
    assert_eq!(indexed.len(), 51);
    assert!(indexed.iter().all(|m| m.id != "transaction-063"));
}
