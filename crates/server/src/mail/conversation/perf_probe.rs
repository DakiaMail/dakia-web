//! Investigation-only probe. Real conversation/page/storage code, synthetic provider.
use super::*;
struct Probe {
    account: String,
    folders: usize,
    count: usize,
    gmail: bool,
    delay: u64,
    scans: AtomicUsize,
    metadata: AtomicUsize,
}
#[async_trait]
impl p::MailProvider for Probe {
    fn account_id(&self) -> &str {
        &self.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        Ok(p::Probe {
            capabilities: vec![],
            folders: (0..self.folders)
                .map(|i| p::Folder {
                    name: format!("F{i}"),
                    selectable: true,
                    attributes: vec![],
                })
                .collect(),
            full_search_coverage: true,
        })
    }
    async fn scan(
        &self,
        folder: &str,
        q: &p::Search,
        token: Option<&p::PageToken>,
        limit: usize,
    ) -> p::Result<p::Page> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        if self.delay > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay)).await;
        }
        let mut rows = vec![];
        for i in (0..self.count).rev() {
            if folder != format!("F{}", i % self.folders) {
                continue;
            }
            let refs = (0..i)
                .map(|j| format!("<m{j}@example.invalid>"))
                .collect::<Vec<_>>()
                .join(" ");
            let raw = raw(&format!("m{i}"), &format!("References: {refs}\r\n"), 28);
            let h = parse(&raw, self.gmail.then_some(123)).unwrap();
            if let Some(id) = &q.conversation_id {
                if !h.ids.contains(id) {
                    continue;
                }
            }
            if q.gmail_thread.is_some_and(|v| v != 123) {
                continue;
            }
            let uid = i as u32 + 1;
            if token.is_some_and(|t| uid >= t.before_uid) {
                continue;
            }
            let r = p::MessageRef {
                account_id: self.account.clone(),
                folder: folder.into(),
                uidvalidity: 1,
                uid,
            };
            rows.push(p::Metadata {
                identity: if self.gmail {
                    p::Identity::Gmail {
                        account_id: self.account.clone(),
                        message_id: uid as u64,
                    }
                } else {
                    p::Identity::Imap(r.clone())
                },
                reference: r,
                gmail_thread_id: self.gmail.then_some(123),
                labels: vec![],
                seen: true,
                starred: false,
                size: raw.len() as u32,
                internal_date: "28-Oct-2026 00:00:00 +0000".into(),
                headers: raw,
            });
        }
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = more.then(|| p::PageToken {
            account_id: self.account.clone(),
            folder: folder.into(),
            uidvalidity: 1,
            upper_uid: self.count as u32,
            before_uid: rows.last().unwrap().reference.uid,
            query: q.clone(),
            modseq: None,
        });
        self.metadata.fetch_add(rows.len(), Ordering::SeqCst);
        Ok(p::Page {
            rows,
            next,
            uidvalidity: 1,
            full_coverage: true,
        })
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
#[ignore = "explicit synthetic performance measurement"]
async fn investigation_thread_scaling() {
    for (folders, count, gmail, delay) in [
        (4, 1, false, 0),
        (4, 10, false, 0),
        (12, 10, false, 0),
        (4, 30, false, 0),
        (4, 10, true, 0),
        (12, 10, true, 0),
        (12, 1, false, 20),
        (12, 10, false, 20),
        (12, 10, true, 20),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path(), [29; 32]).await.unwrap();
        let user = store.user("fixture", "perf").await.unwrap();
        let a = account(&store, &user).await;
        let runtime = MailRuntime::default();
        let provider = Arc::new(Probe {
            account: a.id().into(),
            folders,
            count,
            gmail,
            delay,
            scans: AtomicUsize::new(0),
            metadata: AtomicUsize::new(0),
        });
        runtime
            .providers
            .lock()
            .await
            .insert(a.id().into(), provider.clone());
        store
            .put_provider_document(
                &a,
                "probe",
                &serde_json::to_string(&provider.probe().await.unwrap()).unwrap(),
            )
            .await
            .unwrap();
        runtime.folders(&store, &a).await.unwrap();
        // Populate every folder first: emulate complete header backfill.
        let mut seed = String::new();
        for i in 0..folders {
            let page = runtime
                .page(&store, &a, &format!("F{i}"), p::Search::default(), None)
                .await
                .unwrap();
            if i == 0 {
                seed = page.rows.last().unwrap().id.clone();
            }
        }
        for repeat in 0..4 {
            if repeat == 2 {
                for i in 0..folders {
                    let f = format!("F{i}");
                    let (generation, _) = store.scan_state(&a, &f).await.unwrap().unwrap();
                    let page = runtime
                        .page(&store, &a, &f, p::Search::default(), None)
                        .await
                        .unwrap();
                    store.record_scan(&a, &f, generation, &page).await.unwrap();
                }
            }
            provider.scans.store(0, Ordering::SeqCst);
            provider.metadata.store(0, Ordering::SeqCst);
            let start = Instant::now();
            let mut page = runtime.conversation(&store, &a, &seed, None).await.unwrap();
            let first_ms = start.elapsed().as_secs_f64() * 1000.;
            let first_rows = page.rows.len();
            let mut requests = 1;
            while let Some(token) = page.continuation {
                page = runtime
                    .conversation(&store, &a, &seed, Some(&token))
                    .await
                    .unwrap();
                requests += 1;
                assert!(requests < 1000);
            }
            assert!(page.complete);
            assert_eq!(page.rows.len(), count);
            if repeat >= 2 {
                assert_eq!(requests, 1);
                assert_eq!(provider.scans.load(Ordering::SeqCst), 0);
            }
            println!(
                "PERF {}",
                serde_json::json!({"coverage":if repeat>=2 {"reconciled"} else {"headers_only"},"folders":folders,"messages":count,"gmail":gmail,"delay_ms":delay,"repeat":repeat,"first_ms":first_ms,"total_ms":start.elapsed().as_secs_f64()*1000.,"first_rows":first_rows,"api_requests":requests,"provider_scans":provider.scans.load(Ordering::SeqCst),"metadata_rows":provider.metadata.load(Ordering::SeqCst)})
            );
        }
    }
}
