//! Release invariants against real SQLite, with fault injection at commit boundaries.
use super::*;

#[tokio::test]
async fn completed_history_does_not_restart_every_minute() {
    let (_dir, store, _, a, _) = fixture().await;
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    store
        .record_scan(
            &a,
            "INBOX",
            generation,
            &page(&a, vec![], None, CoverageState::Complete),
        )
        .await
        .unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE folder_scans SET last_scanned_at=last_scanned_at-61 WHERE folder='INBOX'")
        .execute(&db)
        .await
        .unwrap();
    for _ in 0..20 {
        assert!(store.scan_state(&a, "INBOX").await.unwrap().is_none());
    }
    sqlx::query("UPDATE folder_scans SET last_scanned_at=last_scanned_at-240 WHERE folder='INBOX'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        store.scan_state(&a, "INBOX").await.unwrap(),
        Some((generation + 1, None))
    );
}

fn page(
    a: &AuthorizedAccount,
    rows: Vec<Message>,
    next: Option<&str>,
    state: CoverageState,
) -> SearchPage {
    SearchPage {
        rows,
        continuation: next.map(str::to_owned),
        coverage: vec![AccountCoverage {
            account_id: a.id().into(),
            folder_ids: vec!["INBOX".into()],
            inbox: None,
            state,
            error: None,
        }],
        classification: ClassificationCoverage {
            classified: 0,
            total: 0,
        },
        pending: false,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
    }
}

#[tokio::test]
async fn snapshot_failure_rolls_back_cursor_memberships_pruning_and_events() {
    let (_dir, store, _, a, _) = fixture().await;
    let old = store.upsert_message(&a, message(&a, "old")).await.unwrap();
    let new = store.upsert_message(&a, message(&a, "new")).await.unwrap();
    store.seed_inbox_memberships(&a).await.unwrap();
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let db = store.db(&a).await.unwrap();
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&db)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TRIGGER fail_prune BEFORE DELETE ON folder_memberships BEGIN SELECT RAISE(ABORT,'fixture crash'); END;").execute(&db).await.unwrap();
    let end = page(&a, vec![new], None, CoverageState::Complete);
    assert!(store
        .record_scan(&a, "INBOX", generation, &end)
        .await
        .is_err());
    assert_eq!(
        store.scan_state(&a, "INBOX").await.unwrap(),
        Some((generation, None))
    );
    assert_eq!(store.message(&a, "old").await.unwrap(), old);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
            .fetch_one(&db)
            .await
            .unwrap(),
        events
    );
    sqlx::query("DROP TRIGGER fail_prune")
        .execute(&db)
        .await
        .unwrap();
    store
        .record_scan(&a, "INBOX", generation, &end)
        .await
        .unwrap();
    assert!(store
        .message(&a, "old")
        .await
        .unwrap()
        .folder_ids
        .is_empty());
    assert_eq!(store.inbox_rows(&a, None, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn head_commit_failure_preserves_watermark_and_can_replay_after_reopen() {
    let (dir, store, _, a, _) = fixture().await;
    store
        .record_incremental(&a, "INBOX", &[], "before")
        .await
        .unwrap();
    let m = store
        .upsert_message(&a, message(&a, "arrival"))
        .await
        .unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_head BEFORE UPDATE ON provider_documents WHEN NEW.key='sync:head:INBOX' BEGIN SELECT RAISE(ABORT,'fixture crash'); END").execute(&db).await.unwrap();
    assert!(store
        .record_incremental(&a, "INBOX", std::slice::from_ref(&m), "after")
        .await
        .is_err());
    assert_eq!(
        store
            .provider_document(&a, "sync:head:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("before")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM folder_memberships")
            .fetch_one(&db)
            .await
            .unwrap(),
        0
    );
    sqlx::query("DROP TRIGGER fail_head")
        .execute(&db)
        .await
        .unwrap();
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = reopened.authorize(a.user_id(), a.id()).await.unwrap();
    for _ in 0..3 {
        reopened
            .record_incremental(&a, "INBOX", std::slice::from_ref(&m), "after")
            .await
            .unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM folder_memberships")
            .fetch_one(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        reopened
            .provider_document(&a, "sync:head:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("after")
    );
}

macro_rules! conservative_coverage {
    ($name:ident, $state:expr) => {
        #[tokio::test]
        async fn $name() {
            let (_dir, store, _, a, _) = fixture().await;
            let m = store
                .upsert_message(&a, message(&a, "retained"))
                .await
                .unwrap();
            store.seed_inbox_memberships(&a).await.unwrap();
            let (g, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
            store
                .record_scan(&a, "INBOX", g, &page(&a, vec![], Some("resume"), $state))
                .await
                .unwrap();
            store
                .record_scan(
                    &a,
                    "INBOX",
                    g,
                    &page(&a, vec![], None, CoverageState::Complete),
                )
                .await
                .unwrap();
            assert_eq!(
                store.message(&a, &m.id).await.unwrap(),
                m,
                "a later complete page cannot erase earlier uncertainty"
            );
            assert_eq!(
                store.inbox_coverage(&a).await.unwrap().state,
                CoverageState::Partial
            );
        }
    };
}
conservative_coverage!(
    partial_page_latches_until_new_generation,
    CoverageState::Partial
);
conservative_coverage!(error_page_never_authorizes_deletion, CoverageState::Error);
conservative_coverage!(
    pending_page_never_authorizes_deletion,
    CoverageState::Pending
);

#[tokio::test]
async fn stale_generation_cannot_overwrite_new_cursor_or_prune_arrivals() {
    let (_dir, store, _, a, _) = fixture().await;
    let (old, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    store.reset_scan(&a, "INBOX").await.unwrap();
    let (new, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    store
        .record_scan(
            &a,
            "INBOX",
            new,
            &page(&a, vec![], Some("new-cursor"), CoverageState::Complete),
        )
        .await
        .unwrap();
    let arrival = store
        .upsert_message(&a, message(&a, "arrival"))
        .await
        .unwrap();
    store
        .record_incremental(&a, "INBOX", std::slice::from_ref(&arrival), "head")
        .await
        .unwrap();
    assert!(matches!(
        store
            .record_scan(
                &a,
                "INBOX",
                old,
                &page(&a, vec![], None, CoverageState::Complete)
            )
            .await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        store.scan_state(&a, "INBOX").await.unwrap(),
        Some((new, Some("new-cursor".into())))
    );
    store
        .record_scan(
            &a,
            "INBOX",
            new,
            &page(&a, vec![], None, CoverageState::Complete),
        )
        .await
        .unwrap();
    assert_eq!(
        store.inbox_rows(&a, None, None).await.unwrap(),
        vec![arrival]
    );
}

#[tokio::test]
async fn repeated_unchanged_backfill_emits_no_message_events_or_revision_churn() {
    let (_dir, store, _, a, _) = fixture().await;
    let rows: Vec<_> = (0..200).map(|i| message(&a, &format!("m{i:04}"))).collect();
    let first = store.ingest_messages(&a, rows.clone(), &[]).await.unwrap();
    let db = store.db(&a).await.unwrap();
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&db)
        .await
        .unwrap();
    for _ in 0..5 {
        assert_eq!(
            store.ingest_messages(&a, rows.clone(), &[]).await.unwrap(),
            first
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events")
            .fetch_one(&db)
            .await
            .unwrap(),
        events
    );
}

#[tokio::test]
async fn multi_page_keyset_returns_every_message_once_with_tied_timestamps() {
    let (_dir, store, _, a, _) = fixture().await;
    for start in [0, 200, 400] {
        store
            .ingest_messages(
                &a,
                (start..(start + 200).min(513))
                    .map(|i| message(&a, &format!("m{i:04}")))
                    .collect(),
                &[],
            )
            .await
            .unwrap();
    }
    let mut before: Option<(String, String)> = None;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let rows = store
            .inbox_rows(
                &a,
                before
                    .as_ref()
                    .map(|(date, id)| (date.as_str(), id.as_str())),
                None,
            )
            .await
            .unwrap();
        let more = rows.len() > 50;
        for row in rows.iter().take(50) {
            assert!(seen.insert(row.id.clone()), "duplicate {}", row.id);
        }
        if !more {
            break;
        }
        before = Some((rows[49].received_at.clone(), rows[49].id.clone()));
    }
    assert_eq!(seen.len(), 513);
}

#[tokio::test]
async fn concurrent_mutation_claims_have_exactly_one_winner() {
    let (_dir, store, _, a, _) = fixture().await;
    let m = store
        .upsert_message(&a, message(&a, "claim"))
        .await
        .unwrap();
    store
        .operation(
            &a,
            Operation {
                id: "once".into(),
                account_id: a.id().into(),
                target_id: m.id,
                expected_revision: m.revision,
                change: Change::Trash,
                status: OperationStatus::Pending,
                revision: 0,
            },
        )
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let (s, a) = (store.clone(), a.clone());
        tasks.spawn(async move { s.claim_mutation(&a).await.unwrap() });
    }
    let mut winners = 0;
    while let Some(result) = tasks.join_next().await {
        winners += usize::from(result.unwrap().is_some());
    }
    assert_eq!(winners, 1);
    store.recover_mutations(&a).await.unwrap();
    assert!(store.claim_mutation(&a).await.unwrap().is_none());
    assert_eq!(
        store.operation_status(&a, "once").await.unwrap().status,
        OperationStatus::Failed
    );
}

#[tokio::test]
async fn pending_flag_intents_survive_replayed_metadata_and_latest_intent_wins() {
    let (_dir, store, _, a, _) = fixture().await;
    let original = store
        .upsert_message(&a, message(&a, "flags"))
        .await
        .unwrap();
    for (i, change) in [
        Change::Read { value: true },
        Change::Star { value: true },
        Change::Read { value: false },
        Change::Star { value: false },
    ]
    .into_iter()
    .enumerate()
    {
        let m = store.message(&a, &original.id).await.unwrap();
        store
            .operation(
                &a,
                Operation {
                    id: format!("intent-{i}"),
                    account_id: a.id().into(),
                    target_id: m.id,
                    expected_revision: m.revision,
                    change,
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
    }
    let before = store.message(&a, &original.id).await.unwrap();
    assert!(before.unread && !before.starred);
    assert_eq!(store.upsert_message(&a, original).await.unwrap(), before);
}

#[tokio::test]
async fn received_cache_uses_current_flags_and_missing_blob_is_a_cache_miss() {
    let (_dir, store, _, a, _) = fixture().await;
    let m = store.upsert_message(&a, message(&a, "body")).await.unwrap();
    let content: MessageContent = serde_json::from_value(serde_json::json!({"message":m,"text":"synthetic body","html":"","attachments":[],"message_id":null,"references":[],"cc":[],"reply_to":[]})).unwrap();
    store.cache_content(&a, &content, 10_000).await.unwrap();
    let mut newer = m.clone();
    newer.starred = true;
    let newer = store.upsert_message(&a, newer).await.unwrap();
    assert_eq!(
        store
            .cached_content(&a, &m.id)
            .await
            .unwrap()
            .unwrap()
            .message,
        newer
    );
    let db = store.db(&a).await.unwrap();
    let blob: String = sqlx::query_scalar("SELECT blob_id FROM received_cache")
        .fetch_one(&db)
        .await
        .unwrap();
    tokio::fs::remove_file(store.root.join("cache").join(a.id()).join(blob))
        .await
        .unwrap();
    assert!(store.cached_content(&a, &m.id).await.unwrap().is_none());
}

#[tokio::test]
async fn cache_database_failure_does_not_leak_untracked_body_files() {
    let (_dir, store, _, a, _) = fixture().await;
    let m = store.upsert_message(&a, message(&a, "body")).await.unwrap();
    let content: MessageContent = serde_json::from_value(serde_json::json!({"message":m,"text":"synthetic body","html":"","attachments":[],"message_id":null,"references":[],"cc":[],"reply_to":[]})).unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_cache BEFORE INSERT ON received_cache BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END").execute(&db).await.unwrap();
    for _ in 0..3 {
        assert!(store.cache_content(&a, &content, 10_000).await.is_err());
    }
    let path = store.root.join("cache").join(a.id());
    let count = std::fs::read_dir(path).unwrap().count();
    assert_eq!(
        count, 0,
        "failed cache transactions must clean up their untracked body files"
    );
}

#[tokio::test]
async fn raw_mime_shares_cache_budget_ownership_eviction_and_restart_cleanup() {
    let (dir, store, user, a, input) = fixture().await;
    let other = store.connect(&user, &input).await.unwrap();
    let other = store.authorize(&user, &other.id).await.unwrap();
    let m = store.upsert_message(&a, message(&a, "raw")).await.unwrap();
    let content: MessageContent = serde_json::from_value(serde_json::json!({"message":m,"text":"body","html":"","attachments":[],"message_id":null,"references":[],"cc":[],"reply_to":[]})).unwrap();
    let raw = b"From: synthetic@example.invalid\r\n\r\nBinary\x00\xff";
    store
        .cache_content_with_raw(&a, &content, Some(raw), 10_000)
        .await
        .unwrap();
    assert_eq!(
        store.cached_raw(&a, "raw").await.unwrap().as_deref(),
        Some(raw.as_slice())
    );
    assert!(store.cached_raw(&other, "raw").await.unwrap().is_none());
    assert!(store
        .cache_content_with_raw(&other, &content, Some(raw), 10_000)
        .await
        .is_err());
    let db = store.db(&a).await.unwrap();
    let total: i64 = sqlx::query_scalar("SELECT SUM(size_bytes) FROM received_cache")
        .fetch_one(&db)
        .await
        .unwrap();
    let directory = dir.path().join("cache").join(a.id());
    let disk: u64 = std::fs::read_dir(&directory)
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert_eq!(total as u64, disk);
    assert!(disk > raw.len() as u64 + serde_json::to_vec(&content).unwrap().len() as u64);
    let orphan = directory.join(uuid::Uuid::new_v4().to_string());
    std::fs::write(&orphan, b"interrupted write").unwrap();
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a2 = reopened.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        reopened.cached_raw(&a2, "raw").await.unwrap().as_deref(),
        Some(raw.as_slice())
    );
    assert!(!orphan.exists());
    reopened.evict_received(&a2).await.unwrap();
    assert!(reopened.cached_raw(&a2, "raw").await.unwrap().is_none());
    // An oversized entry cannot bypass the configured combined byte budget.
    reopened
        .cache_content_with_raw(&a2, &content, Some(raw), total as u64 - 1)
        .await
        .unwrap();
    assert!(reopened.cached_content(&a2, "raw").await.unwrap().is_none());
    assert!(reopened.cached_raw(&a2, "raw").await.unwrap().is_none());
}

#[tokio::test]
async fn retry_budget_is_durable_and_preserves_per_message_order_without_blocking_other_mail() {
    let (_dir, store, _, a, _) = fixture().await;
    for (id, message_id, change) in [
        ("one", "same", Change::Read { value: true }),
        ("two", "same", Change::Read { value: false }),
        ("other", "other", Change::Archive),
    ] {
        let m = match store.message(&a, message_id).await {
            Ok(m) => m,
            Err(_) => store
                .upsert_message(&a, message(&a, message_id))
                .await
                .unwrap(),
        };
        store
            .operation(
                &a,
                Operation {
                    id: id.into(),
                    account_id: a.id().into(),
                    target_id: m.id,
                    expected_revision: m.revision,
                    change,
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await
            .unwrap();
    }
    assert_eq!(store.claim_mutation(&a).await.unwrap().unwrap().id, "one");
    assert!(store.retry_mutation(&a, "one").await.unwrap());
    let mut other = store.claim_mutation(&a).await.unwrap().unwrap();
    assert_eq!(other.id, "other");
    other.status = OperationStatus::Applied;
    store.finish_mutation(&a, other, None).await.unwrap();
    assert!(store.claim_mutation(&a).await.unwrap().is_none());
    let db = store.db(&a).await.unwrap();
    for remaining in [true, true, false] {
        sqlx::query(
            "UPDATE outbox SET lease_until='2000-01-01T00:00:00.000Z' WHERE target_id='one'",
        )
        .execute(&db)
        .await
        .unwrap();
        let mut first = store.claim_mutation(&a).await.unwrap().unwrap();
        assert_eq!(first.id, "one");
        assert_eq!(store.retry_mutation(&a, "one").await.unwrap(), remaining);
        if !remaining {
            first.status = OperationStatus::Failed;
            store.finish_mutation(&a, first, None).await.unwrap();
        }
    }
    assert_eq!(store.claim_mutation(&a).await.unwrap().unwrap().id, "two");
}

#[tokio::test]
async fn full_snapshot_prunes_untracked_projection_but_partial_snapshot_does_not() {
    for coverage in [CoverageState::Complete, CoverageState::Partial] {
        let (_dir, store, _, a, _) = fixture().await;
        let old = store
            .upsert_message(&a, message(&a, "untracked"))
            .await
            .unwrap();
        let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
        store
            .record_scan(
                &a,
                "INBOX",
                generation,
                &page(&a, vec![], None, coverage.clone()),
            )
            .await
            .unwrap();
        let visible = store.message(&a, &old.id).await.unwrap();
        assert_eq!(
            visible.folder_ids.is_empty(),
            coverage == CoverageState::Complete
        );
    }
}

#[tokio::test]
async fn late_untracked_discovery_is_not_pruned_by_an_older_traversal() {
    let (_dir, store, _, a, _) = fixture().await;
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    store
        .record_scan(
            &a,
            "INBOX",
            generation,
            &page(&a, vec![], Some("next"), CoverageState::Complete),
        )
        .await
        .unwrap();
    // A foreground search discovers a message after this traversal's boundary.
    let arrival = store
        .upsert_message(&a, message(&a, "late-discovery"))
        .await
        .unwrap();
    assert_eq!(
        store.scan_state(&a, "INBOX").await.unwrap(),
        Some((generation, Some("next".into())))
    );
    store
        .record_scan(
            &a,
            "INBOX",
            generation,
            &page(&a, vec![], None, CoverageState::Complete),
        )
        .await
        .unwrap();
    assert!(!store
        .message(&a, &arrival.id)
        .await
        .unwrap()
        .folder_ids
        .is_empty());
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE folder_scans SET last_scanned_at=0 WHERE folder='INBOX'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(store.completed_scan_due(&a, "INBOX").await.unwrap(), None);
    let (next, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    assert_eq!(next, generation + 1);
    store
        .record_scan(
            &a,
            "INBOX",
            next,
            &page(&a, vec![], None, CoverageState::Complete),
        )
        .await
        .unwrap();
    assert!(store
        .message(&a, &arrival.id)
        .await
        .unwrap()
        .folder_ids
        .is_empty());
}
