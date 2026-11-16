//! Lexical backfill continues even when the embedding model cannot load.
use crate::App;
use std::time::{Duration, Instant};
pub async fn run(app: App) {
    let mut cursor = 0usize;
    let mut turns = 0u64;
    loop {
        let mut worked = false;
        if app.store.jobs_enabled().await.unwrap_or(false) {
            if let Ok(mut accounts) = app.store.indexing_accounts().await {
                let count = accounts.len();
                let start = if count > 0 { cursor % count } else { 0 };
                accounts.rotate_left(start);
                let began = Instant::now();
                for (index, a) in accounts.into_iter().enumerate() {
                    if began.elapsed() > Duration::from_secs(2) {
                        break;
                    }
                    cursor = (start + index + 1) % count;
                    turns = turns.wrapping_add(1);
                    match app.store.advance_search_lexical(&a, turns % 8 == 0).await {
                        Ok(progress) => worked |= progress,
                        Err(_) => {
                            tracing::warn!(target:"dakia_server::diagnostics",operation="search_lexical",outcome="failed",account_key=crate::diagnostics::correlation(a.id()),"lexical indexing attempt failed")
                        }
                    }
                    let _ = app.store.flush_embedding_receipts(&a).await;
                }
            }
        }
        tokio::time::sleep(if worked {
            Duration::from_millis(20)
        } else {
            Duration::from_secs(1)
        })
        .await;
    }
}
