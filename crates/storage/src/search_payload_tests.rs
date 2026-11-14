use super::*;
async fn payload_contract(url: Option<&str>) {
    let (dir, mut store, user, a, _) = fixture().await;
    store.configure_search_database(url).await.unwrap();
    // Connection startup is deliberately asynchronous. This persistence
    // contract starts after readiness; outage/startup fallback has its own test.
    store.maintain_search_vectors(false).await.unwrap();
    let m = store
        .upsert_message(&a, message(&a, "payload"))
        .await
        .unwrap();
    store.index_search_headers(&a).await.unwrap();
    store
        .put_search_records(
            &a,
            &[SearchContent {
                message: &m,
                text: "persistent body",
                state: "partial",
                participants: "Maya",
                recipients: "carbon@example.invalid",
                attachments: "report.txt\nattachment token",
                attachment_details: r#"[{"filename":"report.txt","state":"ready"}]"#,
            }],
        )
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    let db = store.db(&a).await.unwrap();
    let staged:(i64,String,String,String,String)=sqlx::query_as("SELECT body_payload,body,participants,mime_recipients,attachment_text FROM search_documents WHERE message_id='payload'").fetch_one(&db).await.unwrap();
    assert_eq!(
        staged,
        (
            0,
            String::new(),
            String::new(),
            String::new(),
            String::new()
        )
    );
    let expected_bytes = "persistent body".len() + "report.txt\nattachment token".len();
    assert_eq!(
        store.text_coverage(&a).await.unwrap().bytes,
        expected_bytes as i64
    );
    let mut flags = m.clone();
    flags.unread = false;
    store.upsert_message(&a, flags).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert_eq!(
        store.text_coverage(&a).await.unwrap().bytes,
        expected_bytes as i64
    );
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
    assert_eq!(
        store
            .full_text_rows(
                &a,
                &CachedSearchFilter {
                    text: Some("attachment token"),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .len(),
        1
    );
    // Reopening does not need a second application-side copy of the body.
    let mut reopened = Store::open(dir.path(), [31; 32]).await.unwrap();
    reopened.configure_search_database(url).await.unwrap();
    let scope = reopened.authorize(&user, a.id()).await.unwrap();
    assert_eq!(
        reopened
            .full_text_rows(
                &scope,
                &CachedSearchFilter {
                    text: Some("persistent"),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .len(),
        1
    );

    let target = store.search_database.as_ref().unwrap();
    // Change only the index incarnation, as a repair/replacement would do.
    let target_db = crate::search_backend::SearchDatabase::open(dir.path(), url)
        .await
        .unwrap();
    match &target_db {
        crate::search_backend::SearchDatabase::Sqlite(pool) => {
            sqlx::query("UPDATE index_identity SET value=lower(hex(randomblob(16)))")
                .execute(pool)
                .await
                .unwrap();
        }
        crate::search_backend::SearchDatabase::Postgres(pool) => {
            sqlx::query("UPDATE index_identity SET value=gen_random_uuid()::TEXT")
                .execute(pool)
                .await
                .unwrap();
        }
    }
    store.flush_search_index(&a).await.unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().processed, 0);
    assert_eq!(
        store.next_search_body(&a, false).await.unwrap().unwrap().id,
        m.id
    );
    assert!(store
        .full_text_rows(
            &a,
            &CachedSearchFilter {
                text: Some("persistent"),
                ..Default::default()
            },
            None
        )
        .await
        .unwrap()
        .is_empty());
    store
        .put_search_body(&a, &m, "reacquired body", "ready")
        .await
        .unwrap();
    // Inject a newer staged write exactly after an older receipt is deleted.
    sqlx::query("CREATE TRIGGER payload_race AFTER DELETE ON search_index_outbox WHEN old.message_id='payload' BEGIN UPDATE search_documents SET body_payload=1,body='newer body',body_state='ready' WHERE message_id=old.message_id; END").execute(&db).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let retained: (i64, String) =
        sqlx::query_as("SELECT body_payload,body FROM search_documents WHERE message_id='payload'")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(retained, (1, "newer body".into()));
    sqlx::query("DROP TRIGGER payload_race")
        .execute(&db)
        .await
        .unwrap();
    store.flush_search_index(&a).await.unwrap();
    assert_eq!(
        store
            .full_text_rows(
                &a,
                &CachedSearchFilter {
                    text: Some("newer"),
                    ..Default::default()
                },
                None
            )
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store.text_coverage(&a).await.unwrap().bytes,
        "newer body".len() as i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT length(body) FROM search_documents WHERE message_id='payload'"
        )
        .fetch_one(&db)
        .await
        .unwrap(),
        0
    );
    let before = store.text_coverage(&a).await.unwrap().bytes;
    store.evict_received(&a).await.unwrap();
    assert_eq!(store.text_coverage(&a).await.unwrap().bytes, before);
    let mut gone = m;
    gone.folder_ids.clear();
    store.upsert_message(&a, gone).await.unwrap();
    store.index_search_headers(&a).await.unwrap();
    store.flush_search_index(&a).await.unwrap();
    let counts = target.coverage(a.id()).await.unwrap();
    assert_eq!((counts.headers, counts.bytes), (0, 0));
}
#[tokio::test]
async fn independent_sqlite_payload_release_replay_and_repair() {
    payload_contract(None).await;
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL with pgvector; DAKIA_TEST_POSTGRES_URL"]
async fn independent_postgres_payload_release_replay_and_repair() {
    let url = std::env::var("DAKIA_TEST_POSTGRES_URL").unwrap();
    payload_contract(Some(&url)).await;
}
