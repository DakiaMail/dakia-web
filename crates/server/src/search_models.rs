//! Local passage indexing. Discovery, leases and progress live in search storage;
//! only one bounded inference turn runs at a time and accounts advance fairly.
use crate::App;
use dakia_inference::embedding::{search_model_revision, LocalSearchEmbedder};
use dakia_storage::{AuthorizedAccount, EmbeddedPassage, Error, Store};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
type Model = Arc<Mutex<LocalSearchEmbedder>>;
pub async fn run(app: App) {
    let path = std::env::var("DAKIA_SEARCH_MODEL_DIR")
        .unwrap_or_else(|_| "crates/inference/assets/multilingual-e5-small".into());
    let revision = search_model_revision();
    let mut model: Option<Model> = None;
    let mut next_load = Instant::now();
    let mut prepared = HashMap::new();
    let mut turns = 0u64;
    let mut account_cursor = 0usize;
    let mut next_snapshot = Instant::now();
    loop {
        if model.is_none() && Instant::now() >= next_load {
            let path = path.clone();
            match tokio::task::spawn_blocking(move || LocalSearchEmbedder::from_dir(path)).await {
                Ok(Ok(loaded)) => {
                    model = Some(Arc::new(Mutex::new(loaded)));
                    prepared.clear();
                }
                _ => {
                    tracing::warn!(target:"dakia_server::diagnostics",operation="search_embeddings",outcome="model_unavailable","local search model unavailable");
                    next_load = Instant::now() + Duration::from_secs(30);
                }
            }
        }
        let enabled = app.store.jobs_enabled().await.unwrap_or(false);
        let mut progressed = false;
        // Recovery reads retained vectors only, so paused provider/extraction
        // work does not prevent rebuilding the local accelerator.
        let snapshot = Instant::now() >= next_snapshot;
        match app.store.maintain_search_vectors(snapshot).await {
            Ok(worked) => {
                progressed |= worked;
                if snapshot && !worked {
                    next_snapshot = Instant::now() + Duration::from_secs(30);
                }
            }
            Err(_) => {
                tracing::warn!(target:"dakia_server::diagnostics",operation="search_vectors",outcome="failed","local vector maintenance failed")
            }
        }
        if let Ok(mut accounts) = app.store.indexing_accounts().await {
            let started = Instant::now();
            let count = accounts.len();
            let start = if count > 0 { account_cursor % count } else { 0 };
            accounts.rotate_left(start);
            for (index, a) in accounts.into_iter().enumerate() {
                if started.elapsed() >= Duration::from_secs(2) {
                    break;
                }
                account_cursor = (start + index + 1) % count;
                let available = model.is_some();
                if prepared.get(a.id()) != Some(&available) {
                    if !matches!(
                        tokio::time::timeout(
                            Duration::from_millis(250),
                            app.store.prepare_embeddings(&a, &revision, available)
                        )
                        .await,
                        Ok(Ok(()))
                    ) {
                        continue;
                    }
                    prepared.insert(a.id().to_owned(), available);
                }
                if enabled {
                    if let Some(model) = &model {
                        let _ = tokio::time::timeout(
                            Duration::from_millis(250),
                            app.store.discover_embeddings(&a, &revision),
                        )
                        .await;
                        turns = turns.wrapping_add(1);
                        match execute_turn(&app.store, &a, &revision, model.clone(), turns % 8 == 0)
                            .await
                        {
                            Ok(work) => progressed |= work,
                            Err(_) => {
                                tracing::warn!(target:"dakia_server::diagnostics",operation="search_embeddings",outcome="failed",account_key=crate::diagnostics::correlation(a.id()),"search embedding attempt failed")
                            }
                        }
                    }
                }
                match tokio::time::timeout(
                    Duration::from_millis(250),
                    app.store.flush_embedding_receipts(&a),
                )
                .await
                {
                    Ok(Ok(worked)) => progressed |= worked,
                    _ => {
                        tracing::warn!(target:"dakia_server::diagnostics",operation="search_completion",outcome="failed",account_key=crate::diagnostics::correlation(a.id()),"search completion delivery will retry")
                    }
                }
                if started.elapsed() >= Duration::from_secs(2) {
                    break;
                }
            }
        }
        tokio::time::sleep(if progressed {
            Duration::from_millis(20)
        } else {
            Duration::from_secs(1)
        })
        .await;
    }
}
async fn execute_turn(
    store: &Store,
    a: &AuthorizedAccount,
    revision: &str,
    model: Model,
    oldest: bool,
) -> Result<bool, Error> {
    let turn = tokio::time::timeout(
        Duration::from_millis(250),
        store.claim_embedding(a, revision, oldest),
    )
    .await
    .map_err(|_| Error::Invalid)??;
    let Some(turn) = turn else {
        return Ok(false);
    };
    let text = turn.text.clone();
    let expected_revision = revision.to_owned();
    let result = tokio::task::spawn_blocking(move || -> Result<EmbeddedPassage, Error> {
        if text.trim().is_empty() {
            return Ok(EmbeddedPassage {
                text: text.clone(),
                byte_end: text.len(),
                next_offset: text.len(),
                vector: None,
            });
        }
        let mut model = model.lock().map_err(|_| Error::Invalid)?;
        if model.revision() != expected_revision {
            return Err(Error::Invalid);
        }
        let passage = model
            .next_passage(&text, 0)
            .map_err(|_| Error::Invalid)?
            .ok_or(Error::Invalid)?;
        let vector = model
            .embed_passages(&[&passage.text])
            .map_err(|_| Error::Invalid)?
            .into_iter()
            .next()
            .ok_or(Error::Invalid)?;
        Ok(EmbeddedPassage {
            text: passage.text,
            byte_end: passage.end,
            next_offset: passage.next_offset,
            vector: Some(vector.to_vec()),
        })
    })
    .await;
    match result {
        Ok(Ok(passage)) => match store.commit_embedding(a, &turn, &passage).await {
            Ok(value) => Ok(value),
            Err(error) => {
                let _ = store.defer_embedding(a, &turn, "storage_failed").await;
                Err(error)
            }
        },
        _ => {
            store.defer_embedding(a, &turn, "model_failed").await?;
            Err(Error::Invalid)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use dakia_domain::*;
    async fn worker_contract(url: Option<&str>) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path(), [27; 32]).await.unwrap();
        store.configure_search_database(url).await.unwrap();
        // This contract measures retained-vector work. Cold extension/migration
        // startup is asynchronous and separately tested for provider fallback.
        store.maintain_search_vectors(false).await.unwrap();
        let user = store.user("fixture", "owner").await.unwrap();
        let input:ConnectAccount=serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"owner","password":"fixture-only"})).unwrap();
        let account = store.connect(&user, &input).await.unwrap();
        let a = store.authorize(&user, &account.id).await.unwrap();
        let message:Message=serde_json::from_value(serde_json::json!({"id":"semantic-worker","account_id":a.id(),"thread_id":"thread","provider_id":"stable","folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":"Sender"},"to":[],"subject":"Archive","preview":"","received_at":"2026-11-10T00:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
        store
            .account_state(&a, AccountState::Reconnect)
            .await
            .unwrap();
        assert!(store
            .indexing_accounts()
            .await
            .unwrap()
            .iter()
            .any(|account| account.id() == a.id()));
        let message = store.upsert_message(&a, message).await.unwrap();
        store.index_search_headers(&a).await.unwrap();
        let text=format!("{} the parcel will arrive tomorrow. track the shipment using this delivery number. tailneedle", "garden plants need light and water. ".repeat(1000));
        store
            .put_search_body(&a, &message, &text, "ready")
            .await
            .unwrap();
        store.flush_search_index(&a).await.unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../inference/assets/multilingual-e5-small");
        let model = Arc::new(Mutex::new(LocalSearchEmbedder::from_dir(path).unwrap()));
        let revision = search_model_revision();
        store.prepare_embeddings(&a, &revision, true).await.unwrap();
        store.discover_embeddings(&a, &revision).await.unwrap();
        for _ in 0..100 {
            execute_turn(&store, &a, &revision, model.clone(), true)
                .await
                .unwrap();
            if store.embedding_coverage(&a).await.unwrap().pending == 0 {
                break;
            }
        }
        let coverage = store.embedding_coverage(&a).await.unwrap();
        assert_eq!(coverage.pending, 0);
        assert_eq!(coverage.processed, 1);
        assert_eq!(store.text_coverage(&a).await.unwrap().processed, 0);
        store.flush_embedding_receipts(&a).await.unwrap();
        let acknowledged = store.text_coverage(&a).await.unwrap();
        assert_eq!((acknowledged.processed, acknowledged.backlog), (1, 0));
        let chunks: Vec<(String, Vec<f32>, i64)> = if let Some(url) = url {
            let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
            let target = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect_with(options.options([("search_path", "dakia_search,public")]))
                .await
                .unwrap();
            let rows:Vec<(String,String,i64)>=sqlx::query_as("SELECT text,embedding::TEXT,byte_end FROM chunks WHERE account_id=$1 AND field=1 ORDER BY ordinal").bind(a.id()).fetch_all(&target).await.unwrap();
            target.close().await;
            rows.into_iter()
                .map(|(text, vector, end)| (text, serde_json::from_str(&vector).unwrap(), end))
                .collect()
        } else {
            let target = sqlx::SqlitePool::connect(&format!(
                "sqlite:{}",
                dir.path().join("search/index.sqlite").display()
            ))
            .await
            .unwrap();
            let rows:Vec<(String,Vec<u8>,i64)>=sqlx::query_as("SELECT text,embedding,byte_end FROM chunks WHERE account_id=? AND field=1 ORDER BY ordinal").bind(a.id()).fetch_all(&target).await.unwrap();
            target.close().await;
            rows.into_iter()
                .map(|(text, bytes, end)| {
                    (
                        text,
                        bytes
                            .chunks_exact(4)
                            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                            .collect(),
                        end,
                    )
                })
                .collect()
        };
        assert!(chunks.len() > 2);
        assert!(chunks.last().unwrap().0.contains("tailneedle"));
        assert_eq!(chunks.last().unwrap().2, text.len() as i64);
        let query = model
            .lock()
            .unwrap()
            .embed_query("When will my package be delivered?")
            .unwrap();
        let score = |vector: &[f32]| vector.iter().zip(query).map(|(a, b)| a * b).sum::<f32>();
        assert!(score(&chunks.last().unwrap().1) > score(&chunks[0].1));
        for _ in 0..100 {
            if !store.maintain_search_vectors(true).await.unwrap() {
                break;
            }
        }
        let found = store
            .semantic_search_rows(
                &a,
                &dakia_storage::CachedSearchFilter::default(),
                &revision,
                &query,
            )
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].field, 1);
        assert!(found[0].excerpt.contains("tailneedle"));
        assert!(!dir.path().join("cache").join(a.id()).exists());
        store.remove_account(&a, &input.email).await.unwrap();
    }
    #[tokio::test]
    #[ignore = "requires pinned public E5 assets; run scripts/fetch-search-model.py"]
    async fn real_worker_persists_later_passages_without_mime_retention() {
        worker_contract(None).await;
    }
    #[tokio::test]
    #[ignore = "requires disposable dakia_embedding_test and E5 assets; DAKIA_TEST_EMBEDDING_POSTGRES_URL"]
    async fn postgres_worker_persists_real_embeddings() {
        let url = std::env::var("DAKIA_TEST_EMBEDDING_POSTGRES_URL").unwrap();
        let options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        assert_eq!(options.get_database(), Some("dakia_embedding_test"));
        worker_contract(Some(&url)).await;
    }
}
