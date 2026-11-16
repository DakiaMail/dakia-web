use super::*;
pub(super) async fn verify_postgres_upgrade(
    url: &str,
    a: &AccountRepository,
    b: &AccountRepository,
) {
    let expected = b.thread_neighbors(&["gmail:42".into()]).await.unwrap();
    assert_eq!(expected.len(), 1);
    let Backend::Postgres(db) = &a.backend else {
        panic!("native PostgreSQL fixture required");
    };
    for (trigger, table) in [
        ("thread_message_insert", "messages"),
        ("thread_message_update", "messages"),
        ("thread_message_delete", "messages"),
        ("thread_evidence_insert", "provider_documents"),
        ("thread_evidence_update", "provider_documents"),
        ("thread_evidence_delete", "provider_documents"),
    ] {
        sqlx::query(&format!("DROP TRIGGER {trigger} ON {table}"))
            .execute(db)
            .await
            .unwrap();
    }
    for function in ["thread_message_changed", "thread_evidence_changed"] {
        sqlx::query(&format!("DROP FUNCTION {function}()"))
            .execute(db)
            .await
            .unwrap();
    }
    for table in ["thread_verifications", "thread_edges"] {
        sqlx::query(&format!("DROP TABLE {table}"))
            .execute(db)
            .await
            .unwrap();
    }
    sqlx::query("ALTER TABLE account_locks DROP COLUMN thread_revision")
        .execute(db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version=7")
        .execute(db)
        .await
        .unwrap();
    // Reopen the genuine pre-graph shape while retaining messages, headers,
    // cache records, and later independent migrations.
    let migrated = AccountRepository::postgres(url, &b.owner, &b.account)
        .await
        .unwrap();
    assert_eq!(
        migrated
            .thread_neighbors(&["gmail:42".into()])
            .await
            .unwrap(),
        expected
    );
    assert!(migrated.cache_blob("cache-star").await.unwrap().is_some());
    assert!(!migrated.thread_verified("component", 0).await.unwrap());
}
pub(super) async fn contract(a: &AccountRepository, b: &AccountRepository) {
    graph(a, b).await;
    cache(a, b).await;
    recipients(a, b).await;
}
async fn graph(a: &AccountRepository, b: &AccountRepository) {
    let mut row = message(&a.account, "graph-message");
    row.provider_id = "gmail:graph".into();
    let mut row = a
        .ingest_messages(vec![row], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    let mut other = row.clone();
    other.account_id = b.account.clone();
    b.ingest_messages(vec![other], &[], &[], None)
        .await
        .unwrap();
    let header = r#"{"ids":["root@example.invalid"],"gmail":42}"#;
    a.put_provider_document("thread-headers:graph-message", header)
        .await
        .unwrap();
    let edges = vec!["rfc:root@example.invalid".into(), "gmail:42".into()];
    assert_eq!(a.thread_neighbors(&edges).await.unwrap().len(), 1);
    assert!(b.thread_neighbors(&edges).await.unwrap().is_empty());
    b.put_provider_document("thread-headers:graph-message", header)
        .await
        .unwrap();
    assert_eq!(
        b.thread_neighbors(&edges).await.unwrap()[0].0.account_id,
        b.account
    );
    let revision = a.thread_revision().await.unwrap();
    assert!(a
        .verify_thread_revision("component", revision)
        .await
        .unwrap());
    assert!(a.thread_verified("component", revision).await.unwrap());
    assert!(!b.thread_verified("component", revision).await.unwrap());
    row.unread = false;
    row.starred = true;
    row.subject = "Updated display".into();
    row = a
        .ingest_messages(vec![row], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(a.thread_revision().await.unwrap(), revision);
    assert!(a.thread_verified("component", revision).await.unwrap());
    let displayed = a.thread_neighbors(&edges).await.unwrap();
    assert!(!displayed[0].0.unread);
    assert_eq!(displayed[0].0.subject, "Updated display");
    row.folder_ids = vec!["Archive".into()];
    row = a
        .ingest_messages(vec![row], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    assert!(a.thread_revision().await.unwrap() > revision);
    assert!(!a.thread_verified("component", revision).await.unwrap());
    assert!(!a.verify_thread_revision("stale", revision).await.unwrap());
    a.put_provider_document("thread:folders", r#"{"names":["Archive"],"at":0}"#)
        .await
        .unwrap();
    let stable = a.thread_revision().await.unwrap();
    let at = Utc::now().timestamp();
    a.put_provider_document(
        "thread:folders",
        &serde_json::json!({"names":["Archive"],"at":at}).to_string(),
    )
    .await
    .unwrap();
    assert_eq!(a.thread_revision().await.unwrap(), stable);
    a.put_provider_document("probe", r#"{"full_search_coverage":true,"folders":[]}"#)
        .await
        .unwrap();
    a.put_provider_document("thread:epoch:Archive", "7")
        .await
        .unwrap();
    assert!(!a.thread_coverage_fresh().await.unwrap());
    let (generation, _) = a.scan_state("Archive").await.unwrap().unwrap();
    a.record_scan(
        "Archive",
        generation,
        &page(&a.account, vec![row.clone()], None, CoverageState::Complete),
        &[],
        &[],
    )
    .await
    .unwrap();
    assert!(a.thread_coverage_fresh().await.unwrap());
    let stable = a.thread_revision().await.unwrap();
    a.put_provider_document(
        "probe",
        r#"{"full_search_coverage":true,"folders":[{"name":"Archive"}]}"#,
    )
    .await
    .unwrap();
    assert_eq!(a.thread_revision().await.unwrap(), stable);
    let mut generic = message(&a.account, "graph-generic");
    generic.starred = true;
    generic.folder_ids = vec!["Archive".into()];
    generic.provider_id =
        serde_json::json!({"folder":"Archive","uidvalidity":7,"uid":1}).to_string();
    a.ingest_messages(vec![generic], &[], &[], None)
        .await
        .unwrap();
    a.put_provider_document(
        "thread-headers:graph-generic",
        r#"{"ids":["root@example.invalid"],"gmail":null}"#,
    )
    .await
    .unwrap();
    assert_eq!(a.thread_neighbors(&edges).await.unwrap().len(), 2);
    a.put_provider_document("thread:epoch:Archive", "8")
        .await
        .unwrap();
    assert!(!a.thread_coverage_fresh().await.unwrap());
    assert_eq!(a.thread_neighbors(&edges).await.unwrap()[0].0.id, row.id);
    assert!(a
        .starred_rows(None, None, &ClassificationFilter::default())
        .await
        .unwrap()
        .iter()
        .all(|m| m.id != "graph-generic"));
    let ids = (0..257).map(|n| format!("edge-{n}")).collect::<Vec<_>>();
    a.put_provider_document(
        "thread-headers:graph-message",
        &serde_json::json!({"ids":ids,"gmail":43}).to_string(),
    )
    .await
    .unwrap();
    assert_eq!(
        a.thread_neighbors(&["rfc:edge-255".into()])
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(a
        .thread_neighbors(&["rfc:edge-256".into()])
        .await
        .unwrap()
        .is_empty());
    assert!(a
        .thread_neighbors(&["gmail:42".into()])
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        a.thread_neighbors(&["gmail:43".into()])
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        a.thread_neighbors(&vec!["edge".into(); 257]).await,
        Err(Error::Invalid)
    ));
    a.clear_provider_document("thread-headers:graph-message")
        .await
        .unwrap();
    assert!(a
        .thread_neighbors(&["gmail:43".into()])
        .await
        .unwrap()
        .is_empty());
    let revision = a.thread_revision().await.unwrap();
    for n in 0..130 {
        assert!(a
            .verify_thread_revision(&format!("bounded-{n:03}"), revision)
            .await
            .unwrap());
    }
    let count: i64 = match &a.backend {
        Backend::Sqlite(db) => sqlx::query_scalar("SELECT count(*) FROM thread_verifications")
            .fetch_one(db)
            .await
            .unwrap(),
        Backend::Postgres(db) => {
            sqlx::query_scalar("SELECT count(*) FROM thread_verifications WHERE account_id=$1")
                .bind(&a.account)
                .fetch_one(db)
                .await
                .unwrap()
        }
    };
    assert_eq!(count, 128);
    assert!(a.verify_thread_revision("expiry", revision).await.unwrap());
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE thread_verifications SET checked_at=0 WHERE component='expiry'")
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("UPDATE thread_verifications SET checked_at=0 WHERE account_id=$1 AND component='expiry'").bind(&a.account).execute(db).await.unwrap();
        }
    }
    assert!(!a.thread_verified("expiry", revision).await.unwrap());
}
async fn cache(a: &AccountRepository, b: &AccountRepository) {
    let providers = vec!["graph-cache-provider".into(), "graph-cache-provider".into()];
    let mut writers = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let (a, providers) = (a.clone(), providers.clone());
        writers.spawn(async move { a.message_ids(&providers).await.unwrap() });
    }
    let mut identity = None;
    while let Some(result) = writers.join_next().await {
        let ids = result.unwrap();
        assert_eq!(ids[0], ids[1]);
        if let Some(expected) = &identity {
            assert_eq!(&ids[0], expected);
        } else {
            identity = Some(ids[0].clone());
        }
    }
    assert!(a
        .known_provider_message(&providers[0])
        .await
        .unwrap()
        .is_none());
    let other = b.message_ids(&providers).await.unwrap();
    assert_ne!(&other[0], identity.as_ref().unwrap());
    let mut known = message(&a.account, identity.as_ref().unwrap());
    known.provider_id = providers[0].clone();
    a.ingest_messages(vec![known.clone()], &[], &[], None)
        .await
        .unwrap();
    assert_eq!(
        a.known_provider_message(&providers[0])
            .await
            .unwrap()
            .unwrap()
            .id,
        known.id
    );
    assert!(b
        .known_provider_message(&providers[0])
        .await
        .unwrap()
        .is_none());
    let mut star = message(&a.account, "cache-star");
    star.starred = true;
    star.received_at = "2100-01-01T00:00:00.000Z".into();
    a.ingest_messages(
        vec![
            star.clone(),
            message(&a.account, "cache-normal"),
            message(&a.account, "cache-last"),
        ],
        &[],
        &[],
        None,
    )
    .await
    .unwrap();
    let mut other_star = star.clone();
    other_star.account_id = b.account.clone();
    b.ingest_messages(vec![other_star], &[], &[], None)
        .await
        .unwrap();
    let star_blob = Uuid::new_v4().to_string();
    let normal_blob = Uuid::new_v4().to_string();
    let last_blob = Uuid::new_v4().to_string();
    let before = receipts(a).await;
    assert!(a
        .put_cache(&star.id, &star_blob, 50, 100)
        .await
        .unwrap()
        .is_empty());
    assert!(a
        .put_cache("cache-normal", &normal_blob, 50, 100)
        .await
        .unwrap()
        .is_empty());
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE received_cache SET last_access='2000-01-01T00:00:00Z'")
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query(
                "UPDATE received_cache SET last_access='2000-01-01T00:00:00Z' WHERE account_id=$1",
            )
            .bind(&a.account)
            .execute(db)
            .await
            .unwrap();
        }
    }
    assert_eq!(
        a.put_cache("cache-last", &last_blob, 50, 100)
            .await
            .unwrap(),
        [normal_blob]
    );
    assert_eq!(
        a.cache_blob(&star.id).await.unwrap(),
        Some(star_blob.clone())
    );
    assert!(a.cache_blob("cache-normal").await.unwrap().is_none());
    let replacement = Uuid::new_v4().to_string();
    assert_eq!(
        a.put_cache(&star.id, &replacement, 50, 100).await.unwrap(),
        [star_blob]
    );
    assert!(matches!(
        a.put_cache(&star.id, &Uuid::new_v4().to_string(), 101, 100)
            .await,
        Err(Error::Invalid)
    ));
    assert_eq!(
        a.cache_blob(&star.id).await.unwrap().as_deref(),
        Some(replacement.as_str())
    );
    assert!(a
        .put_cache("missing-message", &Uuid::new_v4().to_string(), 1, 100)
        .await
        .is_err());
    a.touch_cache(&star.id).await.unwrap();
    assert_eq!(
        a.starred_rows(None, None, &ClassificationFilter::default())
            .await
            .unwrap()[0]
            .id,
        star.id
    );
    assert!(a
        .starred_cache_rows("cache-")
        .await
        .unwrap()
        .iter()
        .any(|m| m.id == star.id));
    b.put_cache(&star.id, &replacement, 50, 100).await.unwrap();
    a.evict_received().await.unwrap();
    assert!(a.cache_blobs().await.unwrap().is_empty());
    assert_eq!(
        b.cache_blob(&star.id).await.unwrap().as_deref(),
        Some(replacement.as_str())
    );
    assert_eq!(
        receipts(a).await,
        before,
        "cache eviction must not evict the search index or its outbox"
    );
    // Leave a live cache reference so account removal also exercises its
    // composite message foreign key while preserving the other account's row.
    a.put_cache(&star.id, &Uuid::new_v4().to_string(), 50, 100)
        .await
        .unwrap();
}
async fn recipients(a: &AccountRepository, b: &AccountRepository) {
    let alice = Address {
        email: "Alice@Example.Invalid".into(),
        name: Some("Alice old".into()),
    };
    let bob = Address {
        email: "bob@example.invalid".into(),
        name: Some("Bob".into()),
    };
    for _ in 0..8 {
        a.learn_recipients(
            "sent-one",
            std::slice::from_ref(&alice),
            "2026-01-01T00:00:00Z",
        )
        .await
        .unwrap();
    }
    a.learn_recipients(
        "sent-two",
        std::slice::from_ref(&bob),
        "2026-02-01T00:00:00Z",
    )
    .await
    .unwrap();
    assert_eq!(
        a.recipient_suggestions("").await.unwrap()[0].email,
        bob.email
    );
    let latest = Address {
        email: "alice@example.invalid".into(),
        name: Some("Alice latest".into()),
    };
    a.learn_recipients(
        "sent-three",
        std::slice::from_ref(&latest),
        "2026-01-02T00:00:00Z",
    )
    .await
    .unwrap();
    assert_eq!(
        a.recipient_suggestions("").await.unwrap()[0]
            .name
            .as_deref(),
        Some("Alice latest")
    );
    assert_eq!(a.recipient_suggestions("ALICE").await.unwrap().len(), 1);
    assert!(a.known_recipient(" ALICE@EXAMPLE.INVALID ").await.unwrap());
    assert!(!b.known_recipient(&alice.email).await.unwrap());
    let upper = Address {
        email: "Ö@example.invalid".into(),
        name: None,
    };
    a.learn_recipients("unicode", &[upper], "2026-01-01T00:00:00Z")
        .await
        .unwrap();
    assert!(!a.known_recipient("ö@example.invalid").await.unwrap());
    let invalid = Address {
        email: "not-an-address".into(),
        name: None,
    };
    a.learn_recipients("invalid", &[invalid], "2026-01-01T00:00:00Z")
        .await
        .unwrap();
    assert!(a
        .recipient_suggestions("not-an-address")
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        a.learn_recipients("", &[], "2026-01-01T00:00:00Z").await,
        Err(Error::Invalid)
    ));
    assert!(b.recipient_suggestions("").await.unwrap().is_empty());
}
