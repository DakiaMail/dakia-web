//! Opt-in end-to-end timing through the real TLS IMAP adapter and SQLite.
use super::*;
struct Wire {
    account: String,
    imap: p::imap::ImapProvider,
    prefix: String,
    scans: AtomicUsize,
    rows: AtomicUsize,
}
#[async_trait]
impl p::MailProvider for Wire {
    fn account_id(&self) -> &str {
        &self.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        let mut probe = self.imap.probe().await?;
        probe.folders.retain(|f| f.name.starts_with(&self.prefix));
        Ok(probe)
    }
    async fn scan(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        assert!(f.starts_with(&self.prefix));
        self.scans.fetch_add(1, Ordering::SeqCst);
        let page = self.imap.scan(f, q, t, n).await?;
        assert!(
            page.rows.iter().all(|m| !m.seen),
            "header discovery must remain read-neutral"
        );
        self.rows.fetch_add(page.rows.len(), Ordering::SeqCst);
        Ok(page)
    }
    async fn search(
        &self,
        f: &str,
        q: &p::Search,
        t: Option<&p::PageToken>,
        n: usize,
    ) -> p::Result<p::Page> {
        self.scan(f, q, t, n).await
    }
    async fn fetch(&self, _: &p::MessageRef, _: u32) -> p::Result<Vec<u8>> {
        panic!("no bodies")
    }
    async fn mutate(&self, _: &p::MessageRef, _: p::Mutation) -> p::Result<p::MutationResult> {
        panic!("no mutation")
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        unreachable!()
    }
    async fn submit(
        &self,
        _: &p::mime::PreparedMessage,
        _: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        panic!("no send")
    }
    async fn reconcile_sent(&self, _: &str, _: &[u8]) -> p::Result<p::SentOutcome> {
        panic!("no filing")
    }
}
#[tokio::test]
#[ignore = "explicit disposable loopback Dovecot fixture only"]
async fn disposable_wire_thread_graph_performance() {
    let prefix = std::env::var("DAKIA_THREAD_FIXTURE_PREFIX").unwrap();
    assert!(prefix.starts_with("DakiaGraph-"));
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [37; 32]).await.unwrap();
    let user = store.user("fixture", "wire-graph").await.unwrap();
    let a = account(&store, &user).await;
    let endpoint = p::Endpoint {
        host: "localhost".into(),
        port: std::env::var("DAKIA_FIXTURE_IMAPS_PORT")
            .unwrap()
            .parse()
            .unwrap(),
        security: p::Security::Tls,
    };
    let config = p::Config {
        account_id: a.id().into(),
        primary_address: "fixture@example.test".into(),
        kind: p::ProviderKind::Imap,
        imap: endpoint.clone(),
        smtp: endpoint,
        timeout: Duration::from_secs(10),
        full_imap_exposure_confirmed: true,
        folders: p::FolderMapping::default(),
    };
    let cert = std::fs::read(std::env::var("DAKIA_FIXTURE_CERT").unwrap()).unwrap();
    let imap = p::imap::ImapProvider::new(
        config,
        p::Credentials {
            username: "fixture".into(),
            password: "fixture-only-password".into(),
        },
    )
    .unwrap()
    .with_root(native_tls::Certificate::from_pem(&cert).unwrap())
    .unwrap();
    let provider = Arc::new(Wire {
        account: a.id().into(),
        imap,
        prefix,
        scans: AtomicUsize::new(0),
        rows: AtomicUsize::new(0),
    });
    let runtime = MailRuntime::default();
    runtime
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    let probe = provider.probe().await.unwrap();
    assert_eq!(probe.folders.len(), 4);
    store
        .put_provider_document(&a, "probe", &serde_json::to_string(&probe).unwrap())
        .await
        .unwrap();
    let folders = runtime.folders(&store, &a).await.unwrap();
    let mut seed = String::new();
    for f in &folders {
        let page = runtime
            .page(&store, &a, &f.name, p::Search::default(), None)
            .await
            .unwrap();
        if seed.is_empty() {
            seed = page.rows.last().unwrap().id.clone();
        }
    }
    for phase in ["headers_only", "reconciled"] {
        if phase == "reconciled" {
            for f in &folders {
                let (generation, _) = store.scan_state(&a, &f.name).await.unwrap().unwrap();
                let page = runtime
                    .page(&store, &a, &f.name, p::Search::default(), None)
                    .await
                    .unwrap();
                store
                    .record_scan(&a, &f.name, generation, &page)
                    .await
                    .unwrap();
            }
        }
        for repeat in 0..2 {
            provider.scans.store(0, Ordering::SeqCst);
            provider.rows.store(0, Ordering::SeqCst);
            let start = Instant::now();
            let mut page = runtime.conversation(&store, &a, &seed, None).await.unwrap();
            let first_ms = start.elapsed().as_secs_f64() * 1000.;
            let first_rows = page.rows.len();
            let first_scans = provider.scans.load(Ordering::SeqCst);
            let mut requests = 1;
            while let Some(token) = page.continuation {
                page = runtime
                    .conversation(&store, &a, &seed, Some(&token))
                    .await
                    .unwrap();
                requests += 1;
                assert!(requests < 100);
            }
            assert!(page.complete);
            assert_eq!(page.rows.len(), 10);
            println!(
                "WIRE_PERF {}",
                serde_json::json!({"phase":phase,"repeat":repeat,"first_ms":first_ms,"first_rows":first_rows,"first_scans":first_scans,"total_ms":start.elapsed().as_secs_f64()*1000.,"requests":requests,"scans":provider.scans.load(Ordering::SeqCst),"metadata_rows":provider.rows.load(Ordering::SeqCst)})
            );
        }
    }
}
