//! Existing native-four text and model vectors survive an interrupted upgrade.
use super::*;

fn legacy_chunks(raw: &str) -> Result<Vec<String>> {
    let (mut offset, mut carry, mut skipping, mut chunks) = (0, String::new(), false, vec![]);
    while offset < raw.len() {
        let mut end = (offset + crate::lexical_text::WINDOW).min(raw.len());
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        let step =
            crate::lexical_text::advance(&raw[offset..end], end == raw.len(), skipping, &carry)?;
        assert!(step.consumed > 0);
        offset += step.consumed;
        carry = step.carry;
        skipping = step.skipping;
        chunks.extend(step.chunks);
    }
    Ok(chunks)
}
async fn vectors(db: &SearchDatabase, account: &str) -> Vec<(i64, String)> {
    match db {
        SearchDatabase::Sqlite(pool) => {
            sqlx::query_as("SELECT id,hex(embedding) FROM chunks WHERE account_id=? ORDER BY id")
                .bind(account)
                .fetch_all(pool)
                .await
                .unwrap()
        }
        SearchDatabase::Postgres(pool) => {
            sqlx::query_as("SELECT id,embedding::text FROM chunks WHERE account_id=$1 ORDER BY id")
                .bind(account)
                .fetch_all(pool)
                .await
                .unwrap()
        }
    }
}
async fn profile(db: &SearchDatabase, account: &str) -> (i64, i64, i64) {
    match db {
        SearchDatabase::Sqlite(pool)=>sqlx::query_as("SELECT CAST(revision AS BIGINT),CAST(sweep_done AS BIGINT),CAST(pending AS BIGINT) FROM lexical_accounts WHERE account_id=?").bind(account).fetch_one(pool).await.unwrap(),
        SearchDatabase::Postgres(pool)=>sqlx::query_as("SELECT CAST(revision AS BIGINT),CAST(sweep_done AS BIGINT),CAST(pending AS BIGINT) FROM lexical_accounts WHERE account_id=$1").bind(account).fetch_one(pool).await.unwrap(),
    }
}
async fn close(db: SearchDatabase) {
    match db {
        SearchDatabase::Sqlite(pool) => pool.close().await,
        SearchDatabase::Postgres(pool) => pool.close().await,
    }
}
async fn contract(url: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let old = if let Some(url) = url {
        let bootstrap = PgPoolOptions::new()
            .max_connections(1)
            .connect(url)
            .await
            .unwrap();
        sqlx::raw_sql("CREATE SCHEMA dakia_search;CREATE EXTENSION vector SCHEMA public;CREATE EXTENSION pg_trgm SCHEMA public;").execute(&bootstrap).await.unwrap();
        bootstrap.close().await;
        let options = sqlx::postgres::PgConnectOptions::from_str(url)
            .unwrap()
            .options([("search_path", "dakia_search,public")]);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap();
        let migrations = sqlx::migrate!("./migrations/search/postgres");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                migrations
                    .iter()
                    .filter(|m| m.version < 15)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&pool)
        .await
        .unwrap();
        SearchDatabase::Postgres(pool)
    } else {
        std::fs::create_dir(dir.path().join("search")).unwrap();
        let pool = budget::sqlite_pool()
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(dir.path().join("search/index.sqlite"))
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        let migrations = sqlx::migrate!("./migrations/search/sqlite");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                migrations
                    .iter()
                    .filter(|m| m.version < 22)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&pool)
        .await
        .unwrap();
        SearchDatabase::Sqlite(pool)
    };
    let account = uuid::Uuid::new_v4().to_string();
    let mut vector = vec![0f32; 384];
    vector[0] = 1.;
    let bytes: Vec<_> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
    let json_vector = serde_json::to_string(&vector).unwrap();
    async { transaction!(&old,&account,tx,local,{
        write!(local,tx,"INSERT INTO embedding_accounts(account_id,model_revision,sweep_done) VALUES(?,'retained-model',1)",[&account],"INSERT INTO embedding_accounts(account_id,model_revision,sweep_done) VALUES($1,'retained-model',1)",[&account]);
        for n in 0..10 {
            let id=format!("upgrade-{n:02}");
            let message:Message=serde_json::from_value(serde_json::json!({"id":id,"account_id":account,"thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"maya@example.invalid","name":"Maya"},"to":[],"subject":"Garden care","preview":"","received_at":"2020-01-01T00:00:00.000Z","received_date":"2020-01-01","unread":false,"starred":false,"category":null,"category_status":"pending","revision":1})).unwrap();
            let metadata=serde_json::to_string(&message).unwrap();
            let body=if n==9 {format!("{} lateoldtoken","historical correspondence ".repeat(4000))} else {"historical correspondence".to_owned()};
            write!(local,tx,"INSERT INTO documents(account_id,message_id,version,source,metadata,sender,recipients,subject,body,participants,attachment_text,received_at,state) VALUES(?,?,1,?,?,'maya@example.invalid','','garden care',?,'','','2020-01-01T00:00:00.000Z','ready')",[&account,&id,&id,&metadata,&body],"INSERT INTO documents(account_id,message_id,version,source,metadata,sender,recipients,subject,body,participants,attachment_text,received_at,state) VALUES($1,$2,1,$3,$4::jsonb,'maya@example.invalid','','garden care',$5,'','','2020-01-01T00:00:00.000Z','ready')",[&account,&id,&id,&metadata,&body]);
            for (field,raw) in [(0,"maya@example.invalid"),(2,"garden care"),(3,body.as_str())] {
                for (ordinal,text) in legacy_chunks(raw)?.into_iter().enumerate() {
                    let native=format!("{} {}",crate::lexical_text::native_scope(&account),crate::lexical_text::native_unit(&text));
                    write!(local,tx,"INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) VALUES(?,?,1,4,?,?,?,?,0)",[&account,&id,field,ordinal as i64,&text,&native],"INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) VALUES($1,$2,1,4,$3,$4,$5,$6,FALSE)",[&account,&id,field,ordinal as i64,&text,&native]);
                }
            }
            write!(local,tx,"INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version) VALUES(?,?,?,'retained-model',0,'retained vector',?,1)",[&account,&id,&id,&bytes],"INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version) VALUES($1,$2,$3,'retained-model',0,'retained vector',$4::vector,1)",[&account,&id,&id,&json_vector]);
        }
        write!(local,tx,"DELETE FROM lexical_jobs WHERE account_id=?",[&account],"DELETE FROM lexical_jobs WHERE account_id=$1",[&account]);
        write!(local,tx,"DELETE FROM embedding_jobs WHERE account_id=?",[&account],"DELETE FROM embedding_jobs WHERE account_id=$1",[&account]);
        write!(local,tx,"UPDATE lexical_accounts SET sweep_done=1 WHERE account_id=?",[&account],"UPDATE lexical_accounts SET sweep_done=1 WHERE account_id=$1",[&account]);
        write!(local,tx,"UPDATE documents SET lexical_revision=4,embedding_revision='retained-model',embedding_state='ready' WHERE account_id=?",[&account],"UPDATE documents SET lexical_revision=4,embedding_revision='retained-model',embedding_state='ready' WHERE account_id=$1",[&account]);
        tx.commit().await.unwrap();
        Ok::<_,Error>(())
    }) }.await.unwrap();
    let identity = old.identity().await.unwrap();
    let before = vectors(&old, &account).await;
    assert_eq!(before.len(), 10);
    close(old).await;
    let upgraded = SearchDatabase::open(dir.path(), url).await.unwrap();
    assert_eq!(upgraded.identity().await.unwrap(), identity);
    assert_eq!(vectors(&upgraded, &account).await, before);
    let initial = profile(&upgraded, &account).await;
    assert_eq!(initial.0, 5);
    assert_eq!(initial.1, 0);
    let filter = CachedSearchFilter {
        text: Some("garden"),
        ..Default::default()
    };
    assert_eq!(
        upgraded
            .ranked_rows(&account, &filter, None)
            .await
            .unwrap()
            .len(),
        10,
        "available old postings must remain searchable during backfill"
    );
    assert!(upgraded.lexical_turn(&account, true).await.unwrap());
    assert_eq!(
        profile(&upgraded, &account).await.2,
        2,
        "one turn remains bounded to eight messages"
    );
    let late = CachedSearchFilter {
        text: Some("lateoldtoken"),
        ..Default::default()
    };
    let mut midway = false;
    for _ in 0..12 {
        let position:Option<(i64,i64)>=match &upgraded {
            SearchDatabase::Sqlite(pool)=>sqlx::query_as("SELECT field,byte_offset FROM lexical_jobs WHERE account_id=? AND message_id='upgrade-09'").bind(&account).fetch_optional(pool).await.unwrap(),
            SearchDatabase::Postgres(pool)=>sqlx::query_as("SELECT CAST(field AS BIGINT),byte_offset FROM lexical_jobs WHERE account_id=$1 AND message_id='upgrade-09'").bind(&account).fetch_optional(pool).await.unwrap(),
        };
        if position.is_some_and(|(field, offset)| field == 3 && offset > 0) {
            midway = true;
            break;
        }
        assert!(upgraded.lexical_turn(&account, true).await.unwrap());
    }
    assert!(
        midway,
        "fixture must interrupt during a large body's projection"
    );
    assert_eq!(
        upgraded
            .ranked_rows(&account, &late, None)
            .await
            .unwrap()
            .len(),
        1,
        "later legacy passages survive incremental replacement"
    );
    close(upgraded).await;
    let reopened = SearchDatabase::open(dir.path(), url).await.unwrap();
    while reopened.lexical_turn(&account, true).await.unwrap() {}
    assert_eq!(
        reopened
            .ranked_rows(&account, &filter, None)
            .await
            .unwrap()
            .len(),
        10
    );
    assert_eq!(
        reopened
            .ranked_rows(&account, &late, None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        vectors(&reopened, &account).await,
        before,
        "a lexical upgrade must not re-embed or replace vector identities"
    );
    assert!(
        reopened
            .embedding_coverage(&account)
            .await
            .unwrap()
            .model_available
    );
    close(reopened).await;
}
#[tokio::test]
async fn sqlite_field_projection_upgrade_preserves_vectors_and_resumes() {
    contract(None).await;
}
#[tokio::test]
#[ignore = "requires fresh disposable database; DAKIA_TEST_UPGRADE_POSTGRES_URL"]
async fn postgres_field_projection_upgrade_preserves_vectors_and_resumes() {
    contract(Some(
        &std::env::var("DAKIA_TEST_UPGRADE_POSTGRES_URL").unwrap(),
    ))
    .await;
}
