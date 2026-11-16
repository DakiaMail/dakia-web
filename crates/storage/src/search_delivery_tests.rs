use super::*;

async fn contract(url: Option<&str>) {
    let (_dir, mut store, _user, a, input) = fixture().await;
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let m = store
        .upsert_message(&a, message(&a, "index-contract"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.put_search_records(&a,&[SearchContent {
                has_attachment: Some(true),
        message:&m,text:"the quarterly report is attached",state:"partial",participants:"Maya Person maya@example.invalid",
        recipients:"carbon@example.invalid",attachments:"budget.xlsx\nconfidential project figures",attachment_details:"[{\"filename\":\"budget.xlsx\",\"state\":\"ready\"},{\"filename\":\"scan.pdf\",\"state\":\"needs_ocr\"}]",
    }]).await.unwrap();
    assert!(store.text_coverage(&a).await.unwrap().delivery_pending > 0);
    assert!(store.flush_search_index(&a).await.unwrap());
    assert!(!store.flush_search_index(&a).await.unwrap());
    let coverage = store.text_coverage(&a).await.unwrap();
    assert_eq!(coverage.delivery_pending, 0);
    assert_eq!((coverage.eligible, coverage.processed), (1, 1));
    for text in ["confidential figures", "Maya", "quarterly report"] {
        let rows = store
            .full_text_rows(
                &a,
                &CachedSearchFilter {
                    text: Some(text),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "{text}");
    }
    assert_eq!(
        store
            .full_text_rows(
                &a,
                &CachedSearchFilter {
                    to: Some("carbon"),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .len(),
        1
    );
    let ranked = store
        .full_text_ranked_rows(
            &a,
            &CachedSearchFilter {
                text: Some("confidential figures"),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(ranked.len(), 1);
    let phrase = vec!["confidential figures".to_owned()];
    assert!(store
        .full_text_ranked_rows(
            &a,
            &CachedSearchFilter {
                text: Some("confidential figures"),
                text_units: Some(&phrase),
                ..Default::default()
            },
            None
        )
        .await
        .unwrap()
        .is_empty());
    let phrase = vec!["quarterly report".to_owned()];
    assert_eq!(
        store
            .full_text_ranked_rows(
                &a,
                &CachedSearchFilter {
                    text: Some("quarterly report"),
                    text_units: Some(&phrase),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .len(),
        1
    );
    let (last, score) = &ranked[0];
    assert!(store
        .full_text_ranked_rows(
            &a,
            &CachedSearchFilter {
                text: Some("confidential figures"),
                ..Default::default()
            },
            Some((*score, &last.received_at, &last.id))
        )
        .await
        .unwrap()
        .is_empty());
    // Delivery may run ahead of a dirty metadata row. A flag-only change must
    // republish text even when the header upsert itself is a no-op.
    let mut read = m.clone();
    read.unread = false;
    store.upsert_message(&a, read).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().processed, 0);
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert_eq!(
        store
            .full_text_rows(
                &a,
                &CachedSearchFilter {
                    text: Some("confidential"),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .len(),
        1
    );
    // Current mailbox truth wins immediately, even before a deletion is delivered.
    let mut trashed = m.clone();
    trashed.folder_ids = vec!["INBOX".into(), "Trash".into()];
    store.upsert_message(&a, trashed).await.unwrap();
    assert!(store
        .full_text_rows(
            &a,
            &CachedSearchFilter {
                text: Some("confidential"),
                ..Default::default()
            },
            None
        )
        .await
        .unwrap()
        .is_empty());
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let coverage = store.text_coverage(&a).await.unwrap();
    assert_eq!((coverage.eligible, coverage.processed), (0, 0));
    assert_eq!(
        (coverage.headers, coverage.bodies, coverage.gaps),
        (0, 0, 0)
    );
    // Restoring mail queues acquisition again; old private text must not survive.
    store.upsert_message(&a, m.clone()).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert!(store
        .full_text_rows(
            &a,
            &CachedSearchFilter {
                text: Some("confidential"),
                ..Default::default()
            },
            None
        )
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        store.next_search_body(&a, true).await.unwrap().unwrap().id,
        m.id
    );
    store.remove_account(&a, &input.email).await.unwrap();
}

#[tokio::test]
async fn independent_sqlite_search_delivery_and_exclusion_contract() {
    contract(None).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL with pgvector; DAKIA_TEST_POSTGRES_URL"]
async fn independent_postgres_search_delivery_and_exclusion_contract() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").expect("isolated test database URL");
    contract(Some(&url)).await;
}

#[tokio::test]
async fn independent_index_replay_cannot_resurrect_a_newer_tombstone() {
    let (dir, _store, _user, a, _) = fixture().await;
    let db = crate::search_backend::SearchDatabase::open(dir.path(), None)
        .await
        .unwrap();
    let document = || crate::search_backend::Document {
        message: message(&a, "replay"),
        source: "replay".into(),
        sender: "person".into(),
        recipients: String::new(),
        mime_recipients: String::new(),
        attachment_details: "[]".into(),
        body_payload: true,
        snoozed_until: None,
        subject: "subject".into(),
        body: "private token".into(),
        participants: String::new(),
        attachments: String::new(),
        state: "ready".into(),
    };
    db.deliver(a.id(), "replay", 1, Some(document()))
        .await
        .unwrap();
    db.deliver(a.id(), "replay", 3, None).await.unwrap();
    db.deliver(a.id(), "replay", 2, Some(document()))
        .await
        .unwrap();
    assert!(db
        .rows(
            a.id(),
            &CachedSearchFilter {
                text: Some("private"),
                ..Default::default()
            },
            None
        )
        .await
        .unwrap()
        .is_empty());
}

async fn ranked_keysets(url: Option<&str>) {
    let (_dir, mut store, _user, a, input) = fixture().await;
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let mut messages = Vec::new();
    for n in 0..70 {
        let mut m = message(&a, &format!("rank-{n:03}"));
        if n == 0 {
            m.subject = "project".into();
        }
        messages.push(store.upsert_message(&a, m).await.unwrap());
    }
    store.index_search_headers(&a).await.unwrap();
    let records: Vec<_> = messages
        .iter()
        .map(|m| (m, "project notes", "ready", Some(false)))
        .collect();
    store.put_search_bodies(&a, &records).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let filter = CachedSearchFilter {
        text: Some("project"),
        ..Default::default()
    };
    let mut after: Option<(f64, String, String)> = None;
    let mut found = std::collections::BTreeSet::new();
    let mut first = true;
    loop {
        let page = store
            .full_text_ranked_rows(
                &a,
                &filter,
                after.as_ref().map(|(s, d, i)| (*s, d.as_str(), i.as_str())),
            )
            .await
            .unwrap();
        if first {
            assert_eq!(page[0].0.id, "rank-000");
            first = false;
        }
        let count = page.len();
        for (message, score) in page.into_iter().take(50) {
            assert!(
                found.insert(message.id.clone()),
                "duplicate relevance keyset row"
            );
            after = Some((score, message.received_at, message.id));
        }
        if count <= 50 {
            break;
        }
    }
    assert_eq!(found.len(), 70);
    store.remove_account(&a, &input.email).await.unwrap();
}

#[tokio::test]
async fn sqlite_relevance_keysets_have_no_duplicate_or_missing_rows() {
    ranked_keysets(None).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL with pgvector; DAKIA_TEST_POSTGRES_URL"]
async fn postgres_relevance_keysets_have_no_duplicate_or_missing_rows() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap();
    ranked_keysets(Some(&url)).await;
}

#[tokio::test]
async fn progress_excludes_invalid_epoch_and_requeues_when_epoch_state_changes() {
    let (_dir, mut store, _user, a, _) = fixture().await;
    store.configure_search_database(None).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let mut m = message(&a, "epoch-progress");
    m.provider_id =
        serde_json::json!({"account_id":a.id(),"folder":"INBOX","uidvalidity":1,"uid":1})
            .to_string();
    let m = store.upsert_message(&a, m).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &m, "private prior epoch", "ready", Some(false))
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().processed, 1);
    // Inserting the first epoch fence must revoke old content as well as updating one.
    store
        .put_provider_document(&a, "thread:epoch:INBOX", "2")
        .await
        .unwrap();
    store
        .put_provider_document(&a, "thread:epoch:INBOX", "3")
        .await
        .unwrap();
    assert_eq!(
        store
            .provider_document(&a, "thread:epoch:INBOX")
            .await
            .unwrap()
            .as_deref(),
        Some("3")
    );
    let coverage = store.text_coverage(&a).await.unwrap();
    assert_eq!(
        (coverage.eligible, coverage.processed, coverage.headers),
        (0, 0, 0)
    );
    assert!(store
        .full_text_ranked_rows(
            &a,
            &CachedSearchFilter {
                text: Some("private"),
                ..Default::default()
            },
            None
        )
        .await
        .unwrap()
        .is_empty());
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("DELETE FROM provider_documents WHERE key='thread:epoch:INBOX'")
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().eligible, 1);
    store.index_search_headers(&a).await.unwrap();
    assert_eq!(
        store.next_search_body(&a, true).await.unwrap().unwrap().id,
        m.id
    );
}

#[tokio::test]
async fn independent_mode_removes_corrupt_legacy_fts_from_mailbox_writes() {
    let (_dir, mut store, _user, a, _) = fixture().await;
    let m = store
        .upsert_message(&a, message(&a, "isolated-write"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_body(&a, &m, "private source body", "ready", Some(false))
        .await
        .unwrap();
    let db = store.db(&a).await.unwrap();
    assert!(
        sqlx::query("UPDATE search_fts_data SET block=x'00' WHERE id>10")
            .execute(&db)
            .await
            .unwrap()
            .rows_affected()
            > 0
    );
    store.configure_search_database(None).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let mut removed = m;
    removed.folder_ids.clear();
    store.upsert_message(&a, removed).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let db = store.db(&a).await.unwrap();
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='search_fts')")
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(!exists);
    assert_eq!(store.text_coverage(&a).await.unwrap().eligible, 0);
}

#[tokio::test]
async fn internal_date_upgrade_restarts_history_without_losing_boundaries_or_messages() {
    let (dir, store, user, a, _) = fixture().await;
    let m = store
        .upsert_message(&a, message(&a, "legacy-calendar"))
        .await
        .unwrap();
    assert!(store
        .cached_search_rows(
            &a,
            &CachedSearchFilter {
                since: Some("2026-10-29"),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .is_empty());
    let (generation, _) = store.scan_state(&a, "INBOX").await.unwrap().unwrap();
    let db = store.db(&a).await.unwrap();
    sqlx::query("UPDATE folder_scans SET completed_at=123,last_scanned_at=123,initial_boundary=42 WHERE folder='INBOX'").execute(&db).await.unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version=37")
        .execute(&db)
        .await
        .unwrap();
    drop(store);
    db.close().await;
    let store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = store.authorize(&user, a.id()).await.unwrap();
    let db = store.db(&a).await.unwrap();
    let state: (i64, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT generation,completed_at,initial_boundary FROM folder_scans WHERE folder='INBOX'",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(state, (generation + 1, None, Some(42)));
    assert_eq!(store.message(&a, &m.id).await.unwrap().received_date, None);
    assert_eq!(
        store.scan_state(&a, "INBOX").await.unwrap().unwrap().0,
        generation + 1
    );
    let mut invalid = m.clone();
    invalid.received_date = Some("2026-02-30".into());
    assert!(matches!(
        store.upsert_message(&a, invalid).await,
        Err(Error::Invalid)
    ));
    assert_eq!(store.message(&a, &m.id).await.unwrap().revision, m.revision);
    drop(store);
    db.close().await;
    let store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = store.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        store.scan_state(&a, "INBOX").await.unwrap().unwrap().0,
        generation + 1,
        "restart must not restart the upgrade again"
    );
}

async fn exact_ranking_contract(url: Option<&str>) {
    let (_dir, mut store, _user, a, input) = fixture().await;
    store.configure_search_database(url).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    let mut messages = Vec::new();
    for (id, subject, body) in [
        ("exact-subject", "Launch plan", "other"),
        ("partial-subject", "Launch plan discussion", "other"),
        ("participant", "Discussion", "other"),
        ("separated-subject", "Launch eventual plan", "other"),
        ("phrase-body", "Discussion", "Launch plan"),
        ("separated-body", "Discussion", "Launch eventual plan"),
    ] {
        let mut m = message(&a, id);
        m.subject = subject.into();
        if id == "exact-subject" {
            m.received_at = "2020-01-01T00:00:00.000Z".into();
        }
        let m = store.upsert_message(&a, m).await.unwrap();
        messages.push((m, body));
    }
    store.index_search_headers(&a).await.unwrap();
    let records: Vec<_> = messages
        .iter()
        .map(|(m, body)| SearchContent {
            has_attachment: Some(true),
            message: m,
            text: body,
            state: "ready",
            participants: if m.id == "participant" {
                "Launch Plan launch@example.invalid"
            } else {
                ""
            },
            recipients: if m.id == "participant" {
                "Launch Plan launch@example.invalid"
            } else {
                ""
            },
            attachments: "",
            attachment_details: "[]",
        })
        .collect();
    store.put_search_records(&a, &records).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let filter = CachedSearchFilter {
        text: Some("launch plan"),
        ..Default::default()
    };
    let hits = store
        .full_text_ranked_rows(&a, &filter, None)
        .await
        .unwrap();
    assert_eq!(
        hits.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(),
        [
            "exact-subject",
            "participant",
            "partial-subject",
            "separated-subject",
            "phrase-body",
            "separated-body"
        ]
    );
    for pair in hits.windows(2) {
        assert!(
            pair[0].1 < pair[1].1,
            "ranking evidence must outweigh recency"
        );
    }
    for (i, (m, score)) in hits.iter().enumerate() {
        let tail = store
            .full_text_ranked_rows(&a, &filter, Some((*score, &m.received_at, &m.id)))
            .await
            .unwrap();
        assert_eq!(
            tail.iter().map(|(m, _)| &m.id).collect::<Vec<_>>(),
            hits[i + 1..].iter().map(|(m, _)| &m.id).collect::<Vec<_>>()
        );
    }
    let mut email_messages = Vec::new();
    for (id, email) in [
        ("identity", "maya@example.invalid"),
        ("address-fragment", "maya@example.invalid.suffix"),
    ] {
        let mut m = message(&a, id);
        m.sender.email = email.into();
        m.sender.name = None;
        if id == "identity" {
            m.received_at = "2020-01-01T00:00:00.000Z".into();
        }
        email_messages.push(store.upsert_message(&a, m).await.unwrap());
    }
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_bodies(
            &a,
            &email_messages
                .iter()
                .map(|m| (m, "other", "ready", Some(false)))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let hits = store
        .full_text_ranked_rows(
            &a,
            &CachedSearchFilter {
                text: Some("maya@example.invalid"),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        hits.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(),
        ["identity", "address-fragment"]
    );
    assert!(
        hits[0].1 < hits[1].1,
        "a fragment of another address must not get the identity boost"
    );
    let mut ids = vec![
        "tie-A", "tie-a", "tie-Ä", "tie-2", "tie-10", "tie-_", "tie-!",
    ];
    let mut tied = Vec::new();
    for id in &ids {
        tied.push(store.upsert_message(&a, message(&a, id)).await.unwrap());
    }
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_bodies(
            &a,
            &tied
                .iter()
                .map(|m| (m, "stabletoken", "ready", Some(false)))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let filter = CachedSearchFilter {
        text: Some("stabletoken"),
        ..Default::default()
    };
    let hits = store
        .full_text_ranked_rows(&a, &filter, None)
        .await
        .unwrap();
    ids.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        hits.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(),
        ids,
        "identity ties must use shared binary ordering"
    );
    for (i, (m, score)) in hits.iter().enumerate() {
        let tail = store
            .full_text_ranked_rows(&a, &filter, Some((*score, &m.received_at, &m.id)))
            .await
            .unwrap();
        assert_eq!(
            tail.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(),
            ids[i + 1..]
        );
    }
    store.remove_account(&a, &input.email).await.unwrap();
}
#[tokio::test]
async fn sqlite_exact_subject_participant_phrase_and_keyset_ranking() {
    exact_ranking_contract(None).await;
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL with pgvector; DAKIA_TEST_POSTGRES_URL"]
async fn postgres_exact_subject_participant_phrase_and_keyset_ranking() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap();
    exact_ranking_contract(Some(&url)).await;
}

#[tokio::test]
async fn readable_gap_upgrade_requeues_once_without_dropping_other_content() {
    let (dir, mut store, user, a, _) = fixture().await;
    store.configure_search_database(None).await.unwrap();
    store.maintain_search_vectors(false).await.unwrap();
    for (id, state) in [
        ("old-encrypted", "encrypted"),
        ("old-large", "oversized"),
        ("old-ready", "ready"),
    ] {
        let m = store.upsert_message(&a, message(&a, id)).await.unwrap();
        store.index_search_headers(&a).await.unwrap();
        store
            .put_search_body(
                &a,
                &m,
                if state == "ready" {
                    "retained healthy body"
                } else {
                    ""
                },
                state,
                Some(false),
            )
            .await
            .unwrap();
    }
    store.flush_search_index(&a).await.unwrap();
    assert!(store
        .next_search_bodies(&a, true, 10)
        .await
        .unwrap()
        .is_empty());
    let db = store.db(&a).await.unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version=40")
        .execute(&db)
        .await
        .unwrap();
    drop(store);
    db.close().await;
    let mut store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = store.authorize(&user, a.id()).await.unwrap();
    store.configure_search_database(None).await.unwrap();
    let mut pending: Vec<_> = store
        .next_search_bodies(&a, true, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    pending.sort();
    assert_eq!(pending, ["old-encrypted", "old-large"]);
    assert_eq!(store.text_coverage(&a).await.unwrap().delivery_pending, 2);
    let ready = store.message(&a, "old-ready").await.unwrap();
    store
        .put_search_body(&a, &ready, "retained healthy body", "ready", Some(false))
        .await
        .unwrap();
    let encrypted = store.message(&a, "old-encrypted").await.unwrap();
    store
        .put_search_body(&a, &encrypted, "", "encrypted", Some(false))
        .await
        .unwrap();
    let db = store.db(&a).await.unwrap();
    drop(store);
    db.close().await;
    let store = Store::open(dir.path(), [31; 32]).await.unwrap();
    let a = store.authorize(&user, a.id()).await.unwrap();
    let pending = store.next_search_bodies(&a, true, 10).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "old-large");
}

#[tokio::test]
async fn external_upgrade_does_not_depend_on_legacy_fts_integrity() {
    let dir = tempfile::tempdir().unwrap();
    let db = pool(&dir.path().join("old-account.sqlite")).await.unwrap();
    let mut old = sqlx::migrate!("./migrations/account");
    old.migrations.to_mut().retain(|m| m.version <= 23);
    old.run(&db).await.unwrap();
    for (id, folder) in [("kept", "INBOX"), ("excluded", "Trash")] {
        let doc = serde_json::json!({"id":id,"folder_ids":[folder],"provider_id":"fixture","received_at":"2026-01-01T00:00:00Z"});
        sqlx::query("INSERT INTO messages(id,revision,document) VALUES(?,1,?)")
            .bind(id)
            .bind(doc.to_string())
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO search_documents(message_id,source,sender,recipients,subject,body,body_state) VALUES(?,'fixture','sender','recipient','booking','retained source text','ready')")
            .bind(id).execute(&db).await.unwrap();
    }
    assert!(
        sqlx::query("UPDATE search_fts_data SET block=x'00' WHERE id>10")
            .execute(&db)
            .await
            .unwrap()
            .rows_affected()
            > 0
    );
    search_legacy::prepare(&db, true).await.unwrap();
    sqlx::migrate!("./migrations/account")
        .run(&db)
        .await
        .unwrap();
    search_legacy::configure(&db, true).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM messages")
            .fetch_one(&db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT body FROM search_documents WHERE message_id='kept'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        "retained source text"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM search_documents WHERE message_id='excluded'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT body_bytes FROM search_totals")
            .fetch_one(&db)
            .await
            .unwrap(),
        20
    );
}
