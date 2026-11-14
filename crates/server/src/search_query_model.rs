//! A dedicated, bounded CPU lane for query embeddings. Passage backfill cannot
//! hold its model lock or queue. Loading and retries never run on request tasks.
use dakia_inference::embedding::{Embedding, LocalSearchEmbedder};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
const CAPACITY: usize = 16;
const CACHE: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryModelState {
    Disabled,
    Loading,
    Ready,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QueryModelUnavailable;
struct Request {
    text: String,
    cache: bool,
    response: oneshot::Sender<Result<Embedding, QueryModelUnavailable>>,
}
trait Embedder: Send {
    fn query(&mut self, text: &str) -> Result<Embedding, QueryModelUnavailable>;
}
impl Embedder for LocalSearchEmbedder {
    fn query(&mut self, text: &str) -> Result<Embedding, QueryModelUnavailable> {
        self.embed_query(text).map_err(|_| QueryModelUnavailable)
    }
}
#[derive(Clone)]
pub(crate) struct QueryModel {
    state: Arc<AtomicU8>,
    sender: Option<mpsc::SyncSender<Request>>,
    cache_queries: bool,
}
impl Default for QueryModel {
    fn default() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(0)),
            sender: None,
            cache_queries: true,
        }
    }
}
impl QueryModel {
    pub fn configured(path: PathBuf) -> Self {
        Self::start(
            move || {
                LocalSearchEmbedder::from_dir(&path)
                    .map(|m| Box::new(m) as Box<dyn Embedder>)
                    .map_err(|_| QueryModelUnavailable)
            },
            Duration::from_secs(30),
        )
    }
    fn start(
        load: impl Fn() -> Result<Box<dyn Embedder>, QueryModelUnavailable> + Send + 'static,
        retry: Duration,
    ) -> Self {
        let state = Arc::new(AtomicU8::new(1));
        let (sender, receiver) = mpsc::sync_channel::<Request>(CAPACITY);
        let status = state.clone();
        if std::thread::Builder::new()
            .name("dakia-search-query".into())
            .spawn(move || {
                let mut model = None;
                let mut next_load = Instant::now();
                let mut cache: VecDeque<(String, Embedding)> = VecDeque::new();
                loop {
                    if model.is_none() && Instant::now() >= next_load {
                        status.store(1, Ordering::Release);
                        // Warm the same inference path before advertising readiness.
                        model = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            load().and_then(|mut m| m.query("search").map(|_| m))
                        }))
                        .ok()
                        .and_then(Result::ok);
                        status.store(if model.is_some() { 2 } else { 3 }, Ordering::Release);
                        next_load = Instant::now() + retry;
                    }
                    let request = if model.is_some() {
                        receiver.recv().map_err(|_| ())
                    } else {
                        match receiver
                            .recv_timeout(next_load.saturating_duration_since(Instant::now()))
                        {
                            Ok(r) => Ok(r),
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
                        }
                    };
                    let Ok(request) = request else {
                        break;
                    };
                    if request.response.is_closed() {
                        continue;
                    }
                    if request.cache {
                        if let Some(index) =
                            cache.iter().position(|(text, _)| text == &request.text)
                        {
                            let hit = cache.remove(index).expect("cache position");
                            let _ = request.response.send(Ok(hit.1));
                            cache.push_front(hit);
                            continue;
                        }
                    }
                    let result = match model.as_mut() {
                        Some(m) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            m.query(&request.text)
                        }))
                        .unwrap_or(Err(QueryModelUnavailable)),
                        None => Err(QueryModelUnavailable),
                    };
                    if let Ok(vector) = &result {
                        if request.cache {
                            cache.push_front((request.text, *vector));
                            cache.truncate(CACHE);
                        }
                    } else {
                        model = None;
                        cache.clear();
                        status.store(3, Ordering::Release);
                        next_load = Instant::now() + retry;
                    }
                    let _ = request.response.send(result);
                }
                status.store(3, Ordering::Release);
            })
            .is_err()
        {
            state.store(3, Ordering::Release);
        }
        Self {
            state,
            sender: Some(sender),
            cache_queries: true,
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(vector: Embedding) -> Self {
        struct Fixed(Embedding);
        impl Embedder for Fixed {
            fn query(&mut self, _: &str) -> Result<Embedding, QueryModelUnavailable> {
                Ok(self.0)
            }
        }
        Self::start(move || Ok(Box::new(Fixed(vector))), Duration::from_secs(30))
    }
    #[cfg(test)]
    pub(crate) fn uncached(mut self) -> Self {
        self.cache_queries = false;
        self
    }
    pub fn state(&self) -> QueryModelState {
        match self.state.load(Ordering::Acquire) {
            0 => QueryModelState::Disabled,
            1 => QueryModelState::Loading,
            2 => QueryModelState::Ready,
            _ => QueryModelState::Unavailable,
        }
    }
    pub async fn embed(&self, text: &str) -> Result<Embedding, QueryModelUnavailable> {
        self.request(text, self.cache_queries).await
    }
    async fn request(&self, text: &str, cache: bool) -> Result<Embedding, QueryModelUnavailable> {
        if self.state() != QueryModelState::Ready
            || text.trim().is_empty()
            || text.len() > 4096
            || text.contains('\0')
        {
            return Err(QueryModelUnavailable);
        }
        let (response, receiver) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or(QueryModelUnavailable)?
            .try_send(Request {
                text: text.into(),
                cache,
                response,
            })
            .map_err(|_| QueryModelUnavailable)?;
        tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .map_err(|_| QueryModelUnavailable)?
            .map_err(|_| QueryModelUnavailable)?
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    struct Fixture {
        calls: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }
    impl Embedder for Fixture {
        fn query(&mut self, text: &str) -> Result<Embedding, QueryModelUnavailable> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                return Err(QueryModelUnavailable);
            }
            if text == "hold" {
                std::thread::sleep(Duration::from_millis(150));
            }
            let mut vector = [0.; 384];
            vector[0] = 1.;
            Ok(vector)
        }
    }
    async fn ready(model: &QueryModel) {
        for _ in 0..100 {
            if model.state() == QueryModelState::Ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("query model did not become ready");
    }
    #[tokio::test]
    async fn cache_failures_retry_and_invalid_queries_are_isolated() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let (c, f) = (calls.clone(), fail.clone());
        let model = QueryModel::start(
            move || {
                Ok(Box::new(Fixture {
                    calls: c.clone(),
                    fail: f.clone(),
                }))
            },
            Duration::from_millis(20),
        );
        ready(&model).await;
        model.embed("first").await.unwrap();
        model.embed("first").await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "warmup and one uncached inference"
        );
        model.request("first", false).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        model.clone().uncached().embed("first").await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        model.embed("first").await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            4,
            "the ordinary handle retains its cache policy"
        );
        assert!(model.embed(&"a".repeat(4097)).await.is_err());
        assert!(model.embed("").await.is_err());
        assert!(model.embed("bad\0query").await.is_err());
        fail.store(true, Ordering::SeqCst);
        assert!(model.embed("failure").await.is_err());
        assert_eq!(model.state(), QueryModelState::Unavailable);
        fail.store(false, Ordering::SeqCst);
        ready(&model).await;
        model.embed("recovered").await.unwrap();
        model.embed("hold").await.unwrap(); // A healthy slow inference must finish.
    }
    #[tokio::test]
    async fn queue_is_bounded_and_cancelled_requests_are_skipped() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let model = QueryModel::start(
            move || {
                Ok(Box::new(Fixture {
                    calls: c.clone(),
                    fail: Arc::new(AtomicBool::new(false)),
                }))
            },
            Duration::from_secs(30),
        );
        ready(&model).await;
        let (response, receiver) = oneshot::channel();
        model
            .sender
            .as_ref()
            .unwrap()
            .try_send(Request {
                text: "hold".into(),
                cache: false,
                response,
            })
            .unwrap_or_else(|_| panic!("first request"));
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut cancelled = vec![];
        for _ in 0..CAPACITY {
            let (response, receiver) = oneshot::channel();
            model
                .sender
                .as_ref()
                .unwrap()
                .try_send(Request {
                    text: "obsolete".into(),
                    cache: false,
                    response,
                })
                .unwrap_or_else(|_| panic!("bounded queue"));
            cancelled.push(receiver);
        }
        assert!(model.embed("overflow").await.is_err());
        drop(cancelled);
        drop(receiver);
        tokio::time::sleep(Duration::from_millis(170)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "cancelled queued work must not run inference"
        );
        model.embed("current").await.unwrap();
    }
    #[tokio::test]
    #[ignore = "requires pinned E5 assets; scripts/fetch-search-model.py"]
    async fn real_cpu_queries_are_uncached_and_multilingual() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../inference/assets/multilingual-e5-small");
        let model = QueryModel::configured(path);
        for _ in 0..2000 {
            if model.state() == QueryModelState::Ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(model.state(), QueryModelState::Ready);
        for query in [
            "When will my package arrive?",
            "Millal pakk kohale jõuab?",
            "میرا پارسل کب آئے گا؟",
            "私の荷物はいつ届きますか",
        ] {
            let vector = model.request(query, false).await.unwrap();
            assert!(vector.iter().all(|v| v.is_finite()));
            assert!((vector.iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 0.001);
        }
    }
}
