use super::*;
use std::collections::BTreeSet;

#[tokio::test]
async fn full_text_secure_delete_churn_preserves_inverted_index() {
    let (_dir, store, _user, a, _) = fixture().await;
    let db = store.db(&a).await.unwrap();
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(
        version, "3.54.1",
        "Engine pin must match the verified release"
    );
    let mut seed = 3_240_194_u64;
    let mut next = |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 32) % n
    };
    let words = [
        "synthetic",
        "café",
        "東京",
        "русский",
        "🙂",
        "abcdefghij",
        " ",
        "\n",
        "xyz",
    ];
    for batch in 0..100 {
        let mut tx = db.begin().await.unwrap();
        for _ in 0..100 {
            let id = format!("churn-{}", next(300));
            let m = message(&a, &id);
            sqlx::query("INSERT INTO messages(id,revision,document) VALUES(?,1,?) ON CONFLICT(id) DO NOTHING")
                .bind(&id).bind(serde_json::to_string(&m).unwrap()).execute(&mut *tx).await.unwrap();
            let body = (0..next(200))
                .map(|_| words[next(words.len() as u64) as usize])
                .collect::<Vec<_>>()
                .join(" ");
            if next(10) == 0 {
                sqlx::query("DELETE FROM search_documents WHERE message_id=?")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
            } else {
                sqlx::query("INSERT INTO search_documents(message_id,source,sender,recipients,subject,body) VALUES(?,?,'sender@example.invalid','','fixture',?) ON CONFLICT(message_id) DO UPDATE SET body=excluded.body")
                    .bind(&id).bind(&id).bind(&body).execute(&mut *tx).await.unwrap();
            }
        }
        tx.commit().await.unwrap();
        sqlx::query("INSERT INTO search_fts(search_fts,rank) VALUES('integrity-check',1)")
            .execute(&db)
            .await
            .unwrap_or_else(|e| panic!("FTS integrity after batch {batch}: {e}"));
    }
    for term in ["synthetic", "café", "東京", "русский", "🙂", "abcdefghij"] {
        let expected: Vec<i64> = sqlx::query_scalar(
            "SELECT rowid FROM search_documents WHERE instr(body,?)>0 ORDER BY rowid",
        )
        .bind(term)
        .fetch_all(&db)
        .await
        .unwrap();
        // Three Unicode characters are required for trigram acceleration.
        if term.chars().count() >= 3 {
            let actual: Vec<i64> = sqlx::query_scalar(
                "SELECT rowid FROM search_fts WHERE search_fts MATCH ? ORDER BY rowid",
            )
            .bind(format!("body:\"{term}\""))
            .fetch_all(&db)
            .await
            .unwrap();
            assert_eq!(actual, expected, "term {term}");
        }
    }
}

async fn ids(store: &Store, a: &AuthorizedAccount, q: &str) -> BTreeSet<String> {
    let mut after = None;
    let mut out = BTreeSet::new();
    loop {
        let page = store
            .full_text_rows(
                a,
                &CachedSearchFilter {
                    text: Some(q),
                    ..Default::default()
                },
                after
                    .as_ref()
                    .map(|(d, i): &(String, String)| (d.as_str(), i.as_str())),
            )
            .await
            .unwrap();
        let more = page.len() > 50;
        for m in page.iter().take(50) {
            assert!(out.insert(m.id.clone()), "duplicate page row");
        }
        if !more {
            break;
        }
        let last = &page[49];
        after = Some((last.received_at.clone(), last.id.clone()));
    }
    out
}
#[tokio::test]
async fn full_text_unicode_substrings_filters_restart_purge_and_account_isolation() {
    let (dir, store, user, a, input) = fixture().await;
    for start in [0, 100] {
        store
            .ingest_messages(
                &a,
                (start..start + 100)
                    .map(|i| {
                        let mut m = message(&a, &format!("m{i:03}"));
                        m.subject = format!("Factuur %_ {i}");
                        m.sender.name = Some("ÉCOLE 東京".into());
                        m.starred = i % 2 == 0;
                        m.unread = i % 3 == 0;
                        m.folder_ids = vec![if i % 2 == 0 { "INBOX" } else { "Sent" }.into()];
                        m
                    })
                    .collect(),
                &[],
            )
            .await
            .unwrap();
    }
    store.index_search_headers(&a).await.unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().pending, 200);
    for i in 0..200 {
        let m = store.message(&a, &format!("m{i:03}")).await.unwrap();
        store
            .put_search_body(
                &a,
                &m,
                &format!(
                    "old body café 東京 русский %_ \"quote\" body-token-{}",
                    i % 7
                ),
                "ready",
            )
            .await
            .unwrap();
    }
    for (q, count) in [
        ("CAFÉ", 200),
        ("cafe\u{301}", 200),
        ("東京", 200),
        ("РУССКИЙ", 200),
        ("body-token-3", 29),
        ("%_", 200),
        ("\"quote\"", 200),
        ("not-present", 0),
        ("ody-tok", 200),
    ] {
        assert_eq!(ids(&store, &a, q).await.len(), count, "query {q}");
    }
    let folders = vec!["Sent".to_owned()];
    let filter = CachedSearchFilter {
        from: Some("école"),
        subject: Some("%_"),
        text: Some("café"),
        folders: Some(&folders),
        starred: Some(false),
        seen: Some(false),
        ..Default::default()
    };
    let filtered = store.full_text_rows(&a, &filter, None).await.unwrap();
    assert_eq!(filtered.len(), 33);
    let second = store.connect(&user, &input).await.unwrap();
    let b = store.authorize(&user, &second.id).await.unwrap();
    assert!(ids(&store, &b, "café").await.is_empty());
    let restarted = Store::open(dir.path(), [1; 32]).await.unwrap();
    let reopened = restarted.authorize(&user, a.id()).await.unwrap();
    assert_eq!(ids(&restarted, &reopened, "body-token-3").await.len(), 29);
    assert!(store
        .full_text_rows(&reopened, &Default::default(), None)
        .await
        .is_err());
    let mut deleted = store.message(&a, "m000").await.unwrap();
    deleted.folder_ids.clear();
    store.upsert_message(&a, deleted.clone()).await.unwrap();
    store
        .put_search_body(&a, &deleted, "resurrect", "ready")
        .await
        .unwrap();
    assert_eq!(ids(&store, &a, "café").await.len(), 199);
    assert!(ids(&store, &a, "resurrect").await.is_empty());
    let mut changed = store.message(&a, "m001").await.unwrap();
    changed.provider_id = "replacement".into();
    store.upsert_message(&a, changed).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().pending, 1);
    let db = store.db(&a).await.unwrap();
    sqlx::query("INSERT INTO search_fts(search_fts,rank) VALUES('integrity-check',1)")
        .execute(&db)
        .await
        .unwrap();
    let size: i64 = sqlx::query_scalar("SELECT body_bytes FROM search_totals")
        .fetch_one(&db)
        .await
        .unwrap();
    let actual: i64 =
        sqlx::query_scalar("SELECT SUM(length(CAST(body AS BLOB))) FROM search_documents")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(size, actual);
}
#[tokio::test]
async fn full_text_epoch_gap_retry_and_partial_scans_never_claim_complete() {
    let (_dir, store, _user, a, _) = fixture().await;
    let mut m = message(&a, "epoch");
    m.provider_id =
        serde_json::json!({"account_id":a.id(),"folder":"INBOX","uidvalidity":1,"uid":1})
            .to_string();
    store
        .put_provider_document(&a, "thread:epoch:INBOX", "1")
        .await
        .unwrap();
    let m = store.upsert_message(&a, m).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &m, "secret body", "ready")
        .await
        .unwrap();
    assert_eq!(ids(&store, &a, "secret").await.len(), 1);
    store
        .put_provider_document(&a, "thread:epoch:INBOX", "2")
        .await
        .unwrap();
    assert!(ids(&store, &a, "secret").await.is_empty());
    store
        .put_search_body(&a, &m, "secret", "ready")
        .await
        .unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().headers, 0);
    let m = store.upsert_message(&a, message(&a, "gap")).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &m, "", "encrypted")
        .await
        .unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().gaps, 1);
    assert!(store.next_search_body(&a, false).await.unwrap().is_none());
    assert!(!store
        .search_scans_fresh(&a, &["INBOX".into()])
        .await
        .unwrap());
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let mut page = SearchPage {
        rows: vec![m],
        continuation: None,
        coverage: vec![],
        classification: ClassificationCoverage {
            classified: 0,
            total: 1,
        },
        pending: false,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
    };
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert!(!store
        .search_scans_fresh(&a, &["INBOX".into()])
        .await
        .unwrap());
    store.reset_scan(&a, "INBOX").await.unwrap();
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    page.coverage = vec![AccountCoverage {
        account_id: a.id().into(),
        folder_ids: vec!["INBOX".into()],
        state: CoverageState::Complete,
        error: None,
        inbox: None,
    }];
    store
        .record_scan(&a, "INBOX", generation, &page)
        .await
        .unwrap();
    assert!(store
        .search_scans_fresh(&a, &["INBOX".into()])
        .await
        .unwrap());
    assert!(!store
        .search_scans_fresh(&a, &["INBOX".into(), "Sent".into()])
        .await
        .unwrap());
}

#[tokio::test]
#[ignore = "finite 50000-message full-text benchmark"]
async fn full_text_authoritative_scale_benchmark() {
    let (dir, store, _user, a, _) = fixture().await;
    let db = store.db(&a).await.unwrap();
    let start = std::time::Instant::now();
    let mut tx = db.begin().await.unwrap();
    let mut expected = BTreeSet::new();
    for i in 0..50_000 {
        let id = format!("m{i:06}");
        let m = message(&a, &id);
        sqlx::query("INSERT INTO messages(id,revision,document) VALUES(?,1,?)")
            .bind(&id)
            .bind(serde_json::to_string(&m).unwrap())
            .execute(&mut *tx)
            .await
            .unwrap();
        let body = format!(
            "Synthetic authoritative historical message {i}. {} café 東京 %_ literal. {}",
            "content ".repeat(40),
            if i % 997 == 0 {
                "rare-body-only"
            } else {
                "ordinary"
            }
        );
        if i % 997 == 0 {
            expected.insert(id.clone());
        }
        sqlx::query("INSERT INTO search_documents(message_id,source,sender,recipients,subject,body,body_state,received_at) VALUES(?,?,'fixture@example.invalid','','synthetic',?,'ready',?)").bind(&id).bind(&id).bind(body).bind(&m.received_at).execute(&mut *tx).await.unwrap();
    }
    sqlx::query("DELETE FROM search_dirty")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let indexing_ms = start.elapsed().as_secs_f64() * 1000.;
    let mut timings = Vec::new();
    for q in [
        "rare-body-only",
        "not-present",
        "cafe\u{301}",
        "%_",
        "content",
    ] {
        let start = std::time::Instant::now();
        let page = store
            .full_text_rows(
                &a,
                &CachedSearchFilter {
                    text: Some(q),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        timings.push((q, page.len(), start.elapsed().as_secs_f64() * 1000.));
    }
    let start = std::time::Instant::now();
    assert_eq!(ids(&store, &a, "rare-body-only").await, expected);
    let exhausted_ms = start.elapsed().as_secs_f64() * 1000.;
    let writer_store = store.clone();
    let writer_account = a.clone();
    let writer = tokio::spawn(async move {
        for i in 10_000..10_200 {
            let m = writer_store
                .message(&writer_account, &format!("m{i:06}"))
                .await
                .unwrap();
            let body=format!("Synthetic authoritative historical message {i}. {} café 東京 %_ literal. {} updated", "content ".repeat(40),if i%997==0 {"rare-body-only"} else {"ordinary"});
            writer_store
                .put_search_body(&writer_account, &m, &body, "ready")
                .await
                .unwrap();
        }
    });
    let mut concurrent = Vec::new();
    for _ in 0..20 {
        let start = std::time::Instant::now();
        assert_eq!(
            store
                .full_text_rows(
                    &a,
                    &CachedSearchFilter {
                        text: Some("content"),
                        ..Default::default()
                    },
                    None
                )
                .await
                .unwrap()
                .len(),
            51
        );
        concurrent.push(start.elapsed().as_secs_f64() * 1000.);
    }
    writer.await.unwrap();
    concurrent.sort_by(f64::total_cmp);
    assert_eq!(ids(&store, &a, "rare-body-only").await, expected);
    let coverage = store.text_coverage(&a).await.unwrap();
    let bytes = std::fs::metadata(
        dir.path()
            .join("accounts")
            .join(format!("{}.sqlite", a.id())),
    )
    .unwrap()
    .len();
    println!("FULL_TEXT_BENCH corpus=50000 body_bytes={} db_bytes={bytes} indexing_ms={indexing_ms:.2} first_pages={timings:?} rare_all_pages_ms={exhausted_ms:.2} rare_count={}",coverage.bytes,expected.len());
    println!("FULL_TEXT_CONCURRENT samples=20 body_updates=200 common_p50_ms={:.2} common_p95_ms={:.2} common_p99_ms={:.2}",concurrent[9],concurrent[18],concurrent[19]);
}

#[tokio::test]
async fn full_text_quota_failed_commit_and_account_removal_preserve_boundaries() {
    let (dir, store, user, a, input) = fixture().await;
    let m = store
        .upsert_message(&a, message(&a, "quota"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    let db = store.db(&a).await.unwrap();
    // Model an account at its budget without allocating half a GiB in a test.
    sqlx::query("UPDATE search_totals SET body_bytes=536870912")
        .execute(&db)
        .await
        .unwrap();
    store
        .put_search_body(&a, &m, "over budget", "ready")
        .await
        .unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().gaps, 1);
    assert!(ids(&store, &a, "over budget").await.is_empty());
    sqlx::query("UPDATE search_totals SET body_bytes=0")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER search_fault BEFORE UPDATE ON search_documents BEGIN SELECT RAISE(ABORT,'synthetic disk write failure'); END")
        .execute(&db).await.unwrap();
    assert!(store
        .put_search_body(&a, &m, "uncommitted", "ready")
        .await
        .is_err());
    assert!(ids(&store, &a, "uncommitted").await.is_empty());
    assert_eq!(store.text_coverage(&a).await.unwrap().gaps, 1);
    sqlx::query("DROP TRIGGER search_fault")
        .execute(&db)
        .await
        .unwrap();
    store
        .put_search_body(&a, &m, "private retained text", "ready")
        .await
        .unwrap();
    assert_eq!(ids(&store, &a, "retained").await.len(), 1);
    store.remove_account(&a, &input.email).await.unwrap();
    assert!(store.authorize(&user, a.id()).await.is_err());
    assert!(store.text_coverage(&a).await.is_err());
    for suffix in [".sqlite", ".sqlite-wal", ".sqlite-shm"] {
        assert!(!dir
            .path()
            .join("accounts")
            .join(format!("{}{suffix}", a.id()))
            .exists());
    }
}

#[tokio::test]
async fn full_text_restore_discards_derived_text_and_requires_new_reconciliation() {
    let (dir, store, user, a, _) = fixture().await;
    let m = store
        .upsert_message(&a, message(&a, "private"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &m, "private search-only text", "ready")
        .await
        .unwrap();
    let backup = dir.path().join("snapshot");
    store.backup(&backup).await.unwrap();
    let snapshot = pool(&backup.join("accounts").join(format!("{}.sqlite", a.id())))
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM search_documents")
            .fetch_one(&snapshot)
            .await
            .unwrap(),
        0
    );
    snapshot.close().await;
    assert_eq!(store.text_coverage(&a).await.unwrap().bodies, 1);
    assert!(
        !std::fs::read(backup.join("accounts").join(format!("{}.sqlite", a.id())))
            .unwrap()
            .windows(b"private search-only text".len())
            .any(|w| w == b"private search-only text")
    );
    let restored = Store::restore(&backup, dir.path().join("restored"), [31; 32])
        .await
        .unwrap();
    let scope = restored.authorize(&user, a.id()).await.unwrap();
    assert!(!restored.jobs_enabled().await.unwrap());
    assert_eq!(restored.text_coverage(&scope).await.unwrap().headers, 0);
    assert_eq!(restored.text_coverage(&scope).await.unwrap().dirty, 1);
    assert!(ids(&restored, &scope, "private search-only text")
        .await
        .is_empty());
    assert!(!restored
        .search_scans_fresh(&scope, &["INBOX".into()])
        .await
        .unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("accounts"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[tokio::test]
async fn permanent_search_gaps_do_not_retry_after_time_or_restart_but_changed_sources_do() {
    let (dir, store, user, a, _) = fixture().await;
    let db = store.db(&a).await.unwrap();
    for state in ["oversized", "encrypted", "parse_error", "fetch_error"] {
        let m = store.upsert_message(&a, message(&a, state)).await.unwrap();
        store.index_search_headers(&a).await.unwrap();
        store.put_search_body(&a, &m, "", state).await.unwrap();
    }
    // Simulate arbitrary elapsed hours, including legacy permanent retry_at values.
    sqlx::query("UPDATE search_documents SET retry_at=0")
        .execute(&db)
        .await
        .unwrap();
    let selected = store.next_search_body(&a, false).await.unwrap().unwrap();
    assert_eq!(selected.id, "fetch_error");
    store
        .put_search_body(&a, &selected, "recovered", "ready")
        .await
        .unwrap();
    assert!(store.next_search_body(&a, false).await.unwrap().is_none());
    let mut changed = store.message(&a, "oversized").await.unwrap();
    changed.provider_id = "replacement-identity".into();
    store.upsert_message(&a, changed).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    assert_eq!(
        store.next_search_body(&a, false).await.unwrap().unwrap().id,
        "oversized"
    );
    let reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    let scope = reopened.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        reopened
            .next_search_body(&scope, true)
            .await
            .unwrap()
            .unwrap()
            .id,
        "oversized"
    );
}

#[tokio::test]
async fn quota_gap_waits_for_capacity_then_becomes_eligible_without_identity_change() {
    let (_dir, store, _user, a, _) = fixture().await;
    let m = store
        .upsert_message(&a, message(&a, "quota-recovery"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.put_search_body(&a, &m, "", "quota").await.unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE search_totals SET body_bytes=512*1024*1024 WHERE id=1")
        .execute(&db)
        .await
        .unwrap();
    assert!(store.next_search_body(&a, false).await.unwrap().is_none());
    sqlx::query("UPDATE search_totals SET body_bytes=511*1024*1024 WHERE id=1")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(
        store.next_search_body(&a, false).await.unwrap().unwrap().id,
        m.id
    );
}
