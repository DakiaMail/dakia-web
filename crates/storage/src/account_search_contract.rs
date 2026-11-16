use super::*;
use crate::full_text::SearchContent;
async fn assert_inventory(a: &AccountRepository) {
    let (coverage, _) = a.source_coverage(None, true).await.unwrap();
    let fields = [
        ("headers", "search_documents", "1=1", coverage.headers),
        (
            "bodies",
            "search_documents",
            "body_state='ready'",
            coverage.bodies,
        ),
        (
            "pending",
            "search_documents",
            "body_state='pending'",
            coverage.pending,
        ),
        (
            "gaps",
            "search_documents",
            "body_state NOT IN ('pending','ready')",
            coverage.gaps,
        ),
        (
            "retrying",
            "search_documents",
            "body_state='fetch_error'",
            coverage.retrying,
        ),
        (
            "processed",
            "search_documents",
            "delivered=1 AND body_state NOT IN ('pending','fetch_error','quota')",
            coverage.processed,
        ),
        ("dirty", "search_dirty", "1=1", coverage.dirty),
        (
            "delivery_pending",
            "search_index_outbox",
            "1=1",
            coverage.delivery_pending,
        ),
        (
            "eligible",
            "search_eligible_messages",
            "1=1",
            coverage.eligible,
        ),
    ];
    for (name, table, predicate, actual) in fields {
        let expected: i64 = match &a.backend {
            Backend::Sqlite(db) => {
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"))
                    .fetch_one(db)
                    .await
                    .unwrap()
            }
            Backend::Postgres(db) => sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE account_id=$1 AND {predicate}"
            ))
            .bind(&a.account)
            .fetch_one(db)
            .await
            .unwrap(),
        };
        assert_eq!(actual, expected, "source inventory {name}");
    }
    let expected:i64=match &a.backend {
        Backend::Sqlite(db)=>sqlx::query_scalar("SELECT COALESCE(SUM(length(CAST(body AS BLOB))+length(CAST(attachment_text AS BLOB))),0) FROM search_documents").fetch_one(db).await.unwrap(),
        Backend::Postgres(db)=>sqlx::query_scalar("SELECT COALESCE(SUM(octet_length(body)+octet_length(attachment_text)),0)::BIGINT FROM search_documents WHERE account_id=$1").bind(&a.account).fetch_one(db).await.unwrap(),
    };
    assert_eq!(coverage.bytes, expected);
}
async fn staged(a: &AccountRepository, id: &str) -> Option<(String, String, i64)> {
    match &a.backend {
        Backend::Sqlite(db)=>sqlx::query_as("SELECT body,body_state,retry_at FROM search_documents WHERE message_id=?").bind(id).fetch_optional(db).await.unwrap(),
        Backend::Postgres(db)=>sqlx::query_as("SELECT body,body_state,retry_at FROM search_documents WHERE account_id=$1 AND message_id=$2").bind(&a.account).bind(id).fetch_optional(db).await.unwrap(),
    }
}
async fn reconcile(a: &AccountRepository) {
    for _ in 0..4 {
        a.index_search_headers().await.unwrap();
    }
}
async fn put(a: &AccountRepository, m: &Message, state: &str) -> Result<()> {
    a.put_search_records(&[SearchContent {
        message: m,
        text: "Later passage CAFÉ 東京",
        state,
        participants: "Reply Person reply@example.invalid",
        recipients: "Hidden hidden@example.invalid",
        attachments: "contract.pdf contents",
        attachment_details: "[{\"filename\":\"contract.pdf\"}]",
    }])
    .await
}
pub(super) async fn contract(a: &AccountRepository, b: &AccountRepository) {
    durable_scan_coverage(a, b).await;
    assert_inventory(a).await;
    assert_inventory(b).await;
    let mut incoming = message(&a.account, "search-acquisition");
    incoming.folder_ids = vec!["INBOX".into(), "All Mail".into()];
    incoming.subject = "Acquisition Quarterly".into();
    let m = a
        .ingest_messages(vec![incoming], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    assert!(a
        .cached_search_rows(&CachedSearchFilter {
            subject: Some("QUARTERLY"),
            ..Default::default()
        })
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert!(!b
        .cached_search_rows(&CachedSearchFilter {
            subject: Some("QUARTERLY"),
            ..Default::default()
        })
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert!(!a
        .cached_search_rows(&CachedSearchFilter {
            subject: Some("QUARTERLY"),
            from: Some("absent@example.invalid"),
            ..Default::default()
        })
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert!(!a
        .cached_search_rows(&CachedSearchFilter {
            subject: Some("QUARTERLY"),
            seen: Some(m.unread),
            ..Default::default()
        })
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    // The separately bounded provisional path preserves the same strict filters
    // and account isolation on both application backends.
    for (repository, filter, expected) in [
        (
            a,
            CachedSearchFilter {
                subject: Some("QUARTERLY"),
                ..Default::default()
            },
            true,
        ),
        (
            b,
            CachedSearchFilter {
                subject: Some("QUARTERLY"),
                ..Default::default()
            },
            false,
        ),
        (
            a,
            CachedSearchFilter {
                subject: Some("QUARTERLY"),
                from: Some("absent@example.invalid"),
                ..Default::default()
            },
            false,
        ),
        (
            a,
            CachedSearchFilter {
                subject: Some("QUARTERLY"),
                seen: Some(m.unread),
                ..Default::default()
            },
            false,
        ),
    ] {
        assert_eq!(
            repository
                .cached_search_preview_rows(&filter)
                .await
                .unwrap()
                .iter()
                .any(|row| row.id == m.id),
            expected
        );
    }
    assert!(a.current_search_message(&m.id).await.unwrap().is_none());
    reconcile(a).await;
    assert_inventory(a).await;
    assert_eq!(staged(a, &m.id).await.unwrap().1, "pending");
    assert!(a
        .next_search_bodies(false, 200)
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert!(a
        .next_search_bodies(true, 200)
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert!(matches!(
        a.next_search_bodies(true, 201).await,
        Err(Error::Invalid)
    ));
    assert!(!a.search_scans_complete(&[]).await.unwrap());
    assert!(!a
        .search_scans_complete(&["never-scanned".into()])
        .await
        .unwrap());
    assert!(matches!(put(b, &m, "ready").await, Err(Error::Invalid)));
    put(a, &m, "fetch_error").await.unwrap();
    assert_inventory(a).await;
    let staged_error = staged(a, &m.id).await.unwrap();
    assert!(staged_error.0.is_empty());
    assert_eq!(staged_error.1, "fetch_error");
    assert!(staged_error.2 > Utc::now().timestamp());
    assert!(!a
        .next_search_bodies(false, 200)
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    put(a, &m, "ready").await.unwrap();
    assert_inventory(a).await;
    assert_eq!(staged(a, &m.id).await.unwrap().0, "later passage café 東京");
    // An acknowledged current candidate becomes visible, while pending metadata
    // changes immediately hide it until reconciliation and delivery complete.
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("DELETE FROM search_index_outbox WHERE message_id=?")
                .bind(&m.id)
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("DELETE FROM search_index_outbox WHERE account_id=$1 AND message_id=$2")
                .bind(&a.account)
                .bind(&m.id)
                .execute(db)
                .await
                .unwrap();
        }
    }
    assert_eq!(
        a.current_search_message(&m.id).await.unwrap().unwrap().id,
        m.id
    );
    assert!(b.current_search_message(&m.id).await.unwrap().is_none());
    a.operation(mutation(
        &m,
        "search-acquisition-star",
        Change::Star { value: true },
    ))
    .await
    .unwrap();
    assert!(a.current_search_message(&m.id).await.unwrap().is_none());
    reconcile(a).await;
    assert_inventory(a).await;
    assert_eq!(staged(a, &m.id).await.unwrap().0, "later passage café 東京");
    let before = receipts(a).await;
    a.set_search_excluded_folders(&["Deleted Items".into()])
        .await
        .unwrap();
    assert!(a
        .search_excluded_folders()
        .await
        .unwrap()
        .contains(&"Deleted Items".into()));
    let once = receipts(a).await;
    a.set_search_excluded_folders(&["Deleted Items".into()])
        .await
        .unwrap();
    assert_eq!(once, receipts(a).await);
    assert_ne!(before.len(), 0);
    assert!(!b
        .search_excluded_folders()
        .await
        .unwrap()
        .contains(&"Deleted Items".into()));
    let mut changed = a.message(&m.id).await.unwrap();
    changed.folder_ids.push("deleted items".into());
    let changed = a
        .ingest_messages(vec![changed], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    // Any excluded membership wins, even with All Mail still present.
    a.set_search_excluded_folders(&["Deleted Items".into(), "Junk E-mail".into()])
        .await
        .unwrap();
    assert!(!a
        .cached_search_preview_rows(&CachedSearchFilter::default())
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert!(staged(a, &m.id).await.is_none());
    assert_inventory(a).await;
    reconcile(a).await;
    assert_inventory(a).await;
    put(a, &changed, "ready").await.unwrap();
    assert!(staged(a, &m.id).await.is_none());
    assert_inventory(a).await;
    let mut restored = changed.clone();
    restored.folder_ids = vec!["INBOX".into()];
    let restored = a
        .ingest_messages(vec![restored], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    reconcile(a).await;
    assert_inventory(a).await;
    assert_eq!(staged(a, &m.id).await.unwrap().1, "pending");
    let mut newer = restored.clone();
    newer.provider_id = "replacement-source".into();
    let newer = a
        .ingest_messages(vec![newer], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    reconcile(a).await;
    assert_inventory(a).await;
    put(a, &restored, "ready").await.unwrap();
    assert_eq!(staged(a, &m.id).await.unwrap().1, "pending");
    put(a, &newer, "quota").await.unwrap();
    assert_eq!(staged(a, &m.id).await.unwrap().1, "pending");
    let mut removed = newer.clone();
    removed.folder_ids.clear();
    a.ingest_messages(vec![removed], &[], &[], None)
        .await
        .unwrap();
    put(a, &newer, "ready").await.unwrap();
    assert!(a.current_search_message(&m.id).await.unwrap().is_none());
    assert!(staged(a, &m.id).await.is_none());
    assert_inventory(a).await;
    delivery_contract(a, None).await;
    if let Ok(url) = std::env::var("DAKIA_TEST_POSTGRES_URL") {
        delivery_contract(a, Some(&url)).await;
    }
    preview_window(a, b).await;
}

// Exercise the nonempty 1,024-row boundary on both application backends,
// including date ties, binary identity order and a match just outside the window.
async fn preview_window(a: &AccountRepository, b: &AccountRepository) {
    const DATE: &str = "2500-01-01T00:00:00.000Z";
    // Other contracts retain malformed dates as well as future mail. Their
    // existing order keys still occupy this provisional physical-row window.
    let preceding: i64 = match &a.backend {
        Backend::Sqlite(db) => sqlx::query_scalar(
            "SELECT count(*) FROM messages WHERE json_extract(document,'$.received_at')>?",
        )
        .bind(DATE)
        .fetch_one(db)
        .await
        .unwrap(),
        Backend::Postgres(db) => sqlx::query_scalar(
            "SELECT count(*) FROM messages WHERE account_id=$1 AND received_at>$2",
        )
        .bind(&a.account)
        .bind(DATE)
        .fetch_one(db)
        .await
        .unwrap(),
    };
    assert!((0..1023).contains(&preceding));
    let ids: Vec<_> = (0..1025)
        .map(|n| format!("preview-window-{n:04}"))
        .chain(["preview-window-Z".into(), "preview-window-ä".into()])
        .collect();
    let mut ordered = ids.clone();
    ordered.sort_by(|a, b| b.cmp(a));
    let boundary = &ordered[1023 - preceding as usize];
    let outside = &ordered[1024 - preceding as usize];
    let messages: Vec<_> = ids
        .iter()
        .map(|id| {
            let mut row = message(&a.account, id);
            row.received_at = DATE.into();
            row.subject = if id == boundary || id == outside {
                "Preview window boundary"
            } else {
                "Preview window"
            }
            .into();
            row
        })
        .collect();
    for batch in messages.chunks(200) {
        a.ingest_messages(batch.to_vec(), &[], &[], None)
            .await
            .unwrap();
    }
    let recent = a
        .cached_search_preview_rows(&CachedSearchFilter {
            subject: Some("Preview window"),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        recent.iter().map(|row| &row.id).collect::<Vec<_>>(),
        ordered.iter().take(50).collect::<Vec<_>>()
    );
    let edge = CachedSearchFilter {
        subject: Some("Preview window boundary"),
        ..Default::default()
    };
    let provisional = a.cached_search_preview_rows(&edge).await.unwrap();
    assert_eq!(
        provisional.iter().map(|row| &row.id).collect::<Vec<_>>(),
        vec![boundary]
    );
    assert_eq!(
        a.cached_search_rows(&edge)
            .await
            .unwrap()
            .iter()
            .map(|row| &row.id)
            .collect::<Vec<_>>(),
        vec![boundary, outside]
    );
    assert!(b
        .cached_search_preview_rows(&CachedSearchFilter {
            subject: Some("Preview window"),
            ..Default::default()
        })
        .await
        .unwrap()
        .is_empty());
}

async fn durable_scan_coverage(a: &AccountRepository, b: &AccountRepository) {
    let folder = "search-coverage-contract";
    let folders = [folder.to_owned()];
    // Old completion timestamps are still proofs of the saved generation.
    // Provider epoch changes, resets and incomplete scans revoke that proof.
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("INSERT INTO folder_scans(folder,generation,completed_at,last_scanned_at,uidvalidity) VALUES(?,7,123,123,5)")
                .bind(folder).execute(db).await.unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("INSERT INTO folder_scans(account_id,folder,generation,completed_at,last_scanned_at,uidvalidity) VALUES($1,$2,7,123,123,5)")
                .bind(&a.account).bind(folder).execute(db).await.unwrap();
        }
    }
    let key = format!("thread:epoch:{folder}");
    a.put_provider_document(&key, "5").await.unwrap();
    assert!(a.search_scans_complete(&folders).await.unwrap());
    assert!(a
        .terminal_scan_folders()
        .await
        .unwrap()
        .contains(&folder.to_owned()));
    assert!(!b.search_scans_complete(&folders).await.unwrap());
    a.put_provider_document(&key, "6").await.unwrap();
    assert!(!a.search_scans_complete(&folders).await.unwrap());
    assert!(!a
        .terminal_scan_folders()
        .await
        .unwrap()
        .contains(&folder.to_owned()));
    a.put_provider_document(&key, "5").await.unwrap();
    assert!(a.search_scans_complete(&folders).await.unwrap());
    // Both absent is valid for non-IMAP providers. One absent is not.
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("UPDATE folder_scans SET uidvalidity=NULL WHERE folder=?")
                .bind(folder)
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query(
                "UPDATE folder_scans SET uidvalidity=NULL WHERE account_id=$1 AND folder=$2",
            )
            .bind(&a.account)
            .bind(folder)
            .execute(db)
            .await
            .unwrap();
        }
    }
    assert!(!a.search_scans_complete(&folders).await.unwrap());
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("DELETE FROM provider_documents WHERE key=?")
                .bind(&key)
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("DELETE FROM provider_documents WHERE account_id=$1 AND key=$2")
                .bind(&a.account)
                .bind(&key)
                .execute(db)
                .await
                .unwrap();
        }
    }
    assert!(a.search_scans_complete(&folders).await.unwrap());
    for (sqlite, postgres) in [
        ("coverage_limited=1", "coverage_limited=TRUE"),
        (
            "coverage_limited=0,continuation='next'",
            "coverage_limited=FALSE,continuation='next'",
        ),
        (
            "continuation=NULL,completed_at=NULL,generation=generation+1",
            "continuation=NULL,completed_at=NULL,generation=generation+1",
        ),
    ] {
        match &a.backend {
            Backend::Sqlite(db) => {
                sqlx::query(&format!("UPDATE folder_scans SET {sqlite} WHERE folder=?"))
                    .bind(folder)
                    .execute(db)
                    .await
                    .unwrap();
            }
            Backend::Postgres(db) => {
                sqlx::query(&format!(
                    "UPDATE folder_scans SET {postgres} WHERE account_id=$1 AND folder=$2"
                ))
                .bind(&a.account)
                .bind(folder)
                .execute(db)
                .await
                .unwrap();
            }
        }
        assert!(!a.search_scans_complete(&folders).await.unwrap());
    }
    match &a.backend {
        Backend::Sqlite(db) => {
            sqlx::query("DELETE FROM folder_scans WHERE folder=?")
                .bind(folder)
                .execute(db)
                .await
                .unwrap();
        }
        Backend::Postgres(db) => {
            sqlx::query("DELETE FROM folder_scans WHERE account_id=$1 AND folder=$2")
                .bind(&a.account)
                .bind(folder)
                .execute(db)
                .await
                .unwrap();
        }
    }
}
async fn delivery_contract(a: &AccountRepository, url: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let target = crate::search_backend::SearchConnection::new(dir.path(), url).unwrap();
    target.warm().await.unwrap();
    target.remove_account(&a.account).await.unwrap();
    let mut m = message(&a.account, "native-delivery");
    m.provider_id = Uuid::new_v4().to_string();
    let m = a
        .ingest_messages(vec![m], &[], &[], None)
        .await
        .unwrap()
        .remove(0);
    reconcile(a).await;
    a.requeue_search_index(&target.identity().await.unwrap())
        .await
        .unwrap();
    put(a, &m, "partial").await.unwrap();
    let deliveries = a.search_deliveries().await.unwrap();
    assert!(deliveries
        .iter()
        .any(|(id, _, doc)| id == &m.id && doc.as_ref().is_some_and(|d| d.body_payload)));
    target.deliver_batch(&a.account, &deliveries).await.unwrap();
    // Replay after a lost source ACK must be idempotent in both destinations.
    target.deliver_batch(&a.account, &deliveries).await.unwrap();
    a.acknowledge_search_deliveries(&deliveries).await.unwrap();
    assert_inventory(a).await;
    assert_eq!(staged(a, &m.id).await.unwrap().0, "");
    assert_eq!(staged(a, &m.id).await.unwrap().1, "partial");
    assert!(a.current_search_message(&m.id).await.unwrap().is_some());
    let filter = CachedSearchFilter {
        text: Some("later passage"),
        ..Default::default()
    };
    let rows = target.rows(&a.account, &filter, None).await.unwrap();
    assert!(rows.iter().any(|(row, _)| row.id == m.id));
    // A newer body queued between destination persistence and ACK is retained.
    put(a, &m, "ready").await.unwrap();
    let old = a.search_deliveries().await.unwrap();
    target.deliver_batch(&a.account, &old).await.unwrap();
    a.put_search_records(&[SearchContent {
        message: &m,
        text: "newest delivery",
        state: "ready",
        participants: "",
        recipients: "",
        attachments: "",
        attachment_details: "[]",
    }])
    .await
    .unwrap();
    a.acknowledge_search_deliveries(&old).await.unwrap();
    assert_inventory(a).await;
    assert_eq!(staged(a, &m.id).await.unwrap().0, "newest delivery");
    assert!(a.current_search_message(&m.id).await.unwrap().is_none());
    let newer = a.search_deliveries().await.unwrap();
    target.deliver_batch(&a.account, &newer).await.unwrap();
    a.acknowledge_search_deliveries(&newer).await.unwrap();
    assert_inventory(a).await;
    assert_eq!(staged(a, &m.id).await.unwrap().0, "");
    assert!(a.search_deliveries().await.unwrap().is_empty());
    a.operation(mutation(
        &m,
        &format!("delivery-snooze-{}", Uuid::new_v4()),
        Change::Snooze {
            until: "2099-01-01T00:00:00Z".into(),
        },
    ))
    .await
    .unwrap();
    reconcile(a).await;
    let deliveries = a.search_deliveries().await.unwrap();
    target.deliver_batch(&a.account, &deliveries).await.unwrap();
    a.acknowledge_search_deliveries(&deliveries).await.unwrap();
    let current = a
        .current_search_messages(std::slice::from_ref(&m.id))
        .await
        .unwrap();
    assert_eq!(current.len(), 1);
    assert!(current[0].1);
    let filtered = a
        .cached_search_rows(&CachedSearchFilter {
            hide_snoozed: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!filtered.iter().any(|row| row.id == m.id));
    let filtered = a
        .cached_search_rows(&CachedSearchFilter {
            snoozed_only: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(filtered.iter().any(|row| row.id == m.id));
    assert!(!a
        .cached_search_preview_rows(&CachedSearchFilter {
            hide_snoozed: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert!(a
        .cached_search_preview_rows(&CachedSearchFilter {
            snoozed_only: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .iter()
        .any(|row| row.id == m.id));
    assert_inventory(a).await;
    // Full authorization batches retain Unicode payloads, per-message snoozes
    // and the pending-delivery boundary. Empty input is also a valid batch.
    let incoming: Vec<_> = (0..200)
        .map(|n| {
            let mut row = message(&a.account, &format!("validation-batch-{n:03}"));
            row.subject = format!("CAFÉ 東京 \"{n}\"\nline");
            row
        })
        .collect();
    let saved = a.ingest_messages(incoming, &[], &[], None).await.unwrap();
    reconcile(a).await;
    loop {
        let pending = a.search_deliveries().await.unwrap();
        if pending.is_empty() {
            break;
        }
        target.deliver_batch(&a.account, &pending).await.unwrap();
        a.acknowledge_search_deliveries(&pending).await.unwrap();
    }
    let ids: Vec<_> = saved.iter().map(|row| row.id.clone()).collect();
    let current = a.current_search_messages(&ids).await.unwrap();
    assert_eq!(current.len(), 200);
    assert!(current.iter().all(|(row, snoozed)| !snoozed
        && saved
            .iter()
            .any(|expected| expected.id == row.id && expected.subject == row.subject)));
    assert!(a.current_search_messages(&[]).await.unwrap().is_empty());
    let mut changed = saved[0].clone();
    changed.subject = "Pending newer metadata".into();
    a.ingest_messages(vec![changed], &[], &[], None)
        .await
        .unwrap();
    assert_eq!(a.current_search_messages(&ids).await.unwrap().len(), 199);
    // Incarnation changes reacquire released text rather than claiming coverage.
    let replacement = Uuid::new_v4().to_string();
    a.requeue_search_index(&replacement).await.unwrap();
    assert_eq!(
        a.provider_document("search:database")
            .await
            .unwrap()
            .as_deref(),
        Some(replacement.as_str())
    );
    assert_eq!(staged(a, &m.id).await.unwrap().1, "pending");
    assert_inventory(a).await;
    target.remove_account(&a.account).await.unwrap();
}
