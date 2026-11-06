//! Bounded, account-instance-local raw metadata cache. Never cache API rows:
//! category, snooze and optimistic mutations must still be read from storage.
use crate::*;
use std::{
    collections::HashMap,
    future::Future,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct ScanCache {
    entries: HashMap<String, (Instant, Page)>,
    failure: Option<(Instant, u32, ProviderError)>,
    jitter_ms: u64,
}
impl ScanCache {
    pub(super) async fn uncached<T, F, Fut>(&mut self, load: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        self.check_backoff()?;
        let result = load().await;
        self.record_result(&result);
        result
    }
    fn check_backoff(&self) -> Result<()> {
        if let Some((at, failures, error)) = &self.failure {
            let seconds = (5u64 * 2u64.pow((*failures - 1).min(4))).min(60);
            if at.elapsed() < Duration::from_millis(seconds * 1000 + self.jitter_ms) {
                return Err(error.clone());
            }
        }
        Ok(())
    }
    fn record_result<T>(&mut self, result: &Result<T>) {
        match result {
            Ok(_) => self.failure = None,
            Err(error)
                if matches!(error.kind, ErrorKind::Retryable | ErrorKind::Authentication) =>
            {
                let failures = self
                    .failure
                    .as_ref()
                    .map_or(1, |(_, n, _)| n.saturating_add(1));
                // Account-instance-local bounded retry dispersion, not a shared
                // global sleep. No retry happens inside an interrupted command.
                self.jitter_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as u64
                    % 1000;
                self.failure = Some((Instant::now(), failures, error.clone()));
            }
            Err(_) => {}
        }
    }
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.failure = None;
    }
    pub(super) async fn get<F, Fut>(&mut self, key: String, load: F) -> Result<Page>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Page>>,
    {
        self.entries
            .retain(|_, (at, _)| at.elapsed() < Duration::from_secs(30));
        // A provider outage is account-wide, not a reason to start a fresh
        // connection for each new query or page. Success resets the backoff.
        self.check_backoff()?;
        if let Some((_, page)) = self.entries.get(&key) {
            return Ok(page.clone());
        }
        let result = self.uncached(load).await;
        if let Ok(page) = &result {
            // Metadata headers can be large. Bound both entry count and
            // individual pages; no persistent private search/body cache.
            let bytes: usize = page.rows.iter().map(|m| m.headers.len()).sum();
            if bytes <= 1024 * 1024 {
                if self.entries.len() >= 16 {
                    if let Some(oldest) = self
                        .entries
                        .iter()
                        .min_by_key(|(_, (at, _))| *at)
                        .map(|(k, _)| k.clone())
                    {
                        self.entries.remove(&oldest);
                    }
                }
                self.entries.insert(key, (Instant::now(), page.clone()));
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::sync::Mutex;
    fn page() -> Page {
        Page {
            rows: vec![],
            next: None,
            uidvalidity: 1,
            full_coverage: true,
        }
    }
    #[tokio::test]
    async fn concurrent_requests_share_work_keys_expire_and_accounts_are_isolated() {
        let cache = Arc::new(Mutex::new(ScanCache::default()));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut jobs = vec![];
        for _ in 0..20 {
            let cache = cache.clone();
            let calls = calls.clone();
            jobs.push(tokio::spawn(async move {
                cache
                    .lock()
                    .await
                    .get("inbox-page1".into(), || async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        Ok(page())
                    })
                    .await
                    .unwrap();
            }));
        }
        for job in jobs {
            job.await.unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let mut c = cache.lock().await;
        for key in ["inbox-page2", "sent-page1", "inbox-different-query"] {
            c.get(key.into(), || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(page())
            })
            .await
            .unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        c.entries.get_mut("inbox-page1").unwrap().0 = Instant::now() - Duration::from_secs(31);
        c.get("inbox-page1".into(), || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(page())
        })
        .await
        .unwrap();
        c.clear();
        c.get("inbox-page1".into(), || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(page())
        })
        .await
        .unwrap();
        ScanCache::default()
            .get("inbox-page1".into(), || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(page())
            })
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 7);
    }
    #[tokio::test]
    async fn errors_back_off_across_queries_expire_and_success_recovers() {
        let mut c = ScanCache::default();
        let error = || ProviderError {
            kind: ErrorKind::Retryable,
            message: "fixture".into(),
        };
        assert!(c
            .get("one".into(), || async { Err(error()) })
            .await
            .is_err());
        assert!(c
            .get("two".into(), || async {
                panic!("must not contact provider")
            })
            .await
            .is_err());
        c.failure.as_mut().unwrap().0 = Instant::now() - Duration::from_secs(6);
        assert!(c
            .get("two".into(), || async { Err(error()) })
            .await
            .is_err());
        assert_eq!(c.failure.as_ref().unwrap().1, 2);
        c.failure.as_mut().unwrap().0 = Instant::now() - Duration::from_secs(11);
        c.get("two".into(), || async { Ok(page()) }).await.unwrap();
        assert!(c.failure.is_none());
    }
    #[tokio::test]
    async fn canceled_load_releases_lane_without_poisoning_cache_and_size_is_bounded() {
        let cache = Arc::new(Mutex::new(ScanCache::default()));
        let other = cache.clone();
        let task = tokio::spawn(async move {
            other
                .lock()
                .await
                .get("one".into(), std::future::pending)
                .await
        });
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        let mut c = cache.lock().await;
        for i in 0..50 {
            c.get(i.to_string(), || async { Ok(page()) }).await.unwrap();
        }
        assert_eq!(c.entries.len(), 16);
        assert!(c.failure.is_none());
    }
}
