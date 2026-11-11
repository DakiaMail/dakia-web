use super::*;
use async_trait::async_trait;
use p::MailProvider;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[path = "graph_tests.rs"]
mod graph_tests;
#[path = "perf_probe.rs"]
mod perf_probe;
#[path = "refresh_tests.rs"]
mod refresh_tests;
#[path = "wire_probe.rs"]
mod wire_probe;

struct Provider {
    account: String,
    rows: Vec<(String, Vec<u8>, u32)>,
    fail: AtomicBool,
    scans: AtomicUsize,
    delay_ms: u64,
    gmail: bool,
}
fn raw(id: &str, extra: &str, day: u32) -> Vec<u8> {
    format!("From: sender@example.invalid\r\nTo: owner@example.invalid\r\nSubject: Floor heating\r\nDate: {day} Oct 2026 10:00:00 +0000\r\nMessage-ID: <{id}@example.invalid>\r\n{extra}\r\nFixture body never needed for discovery\r\n").into_bytes()
}
fn folders() -> Vec<p::Folder> {
    ["INBOX", "Sent Items", "Archive", "Empty"]
        .into_iter()
        .map(|name| p::Folder {
            name: name.into(),
            selectable: true,
            attributes: if name == "Sent Items" {
                vec!["Sent".into()]
            } else {
                vec![]
            },
        })
        .collect()
}
#[async_trait]
impl p::MailProvider for Provider {
    fn account_id(&self) -> &str {
        &self.account
    }
    async fn probe(&self) -> p::Result<p::Probe> {
        Ok(p::Probe {
            capabilities: vec![],
            folders: folders(),
            full_search_coverage: true,
        })
    }
    async fn scan(
        &self,
        folder: &str,
        q: &p::Search,
        token: Option<&p::PageToken>,
        _: usize,
    ) -> p::Result<p::Page> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(p::error(p::ErrorKind::Retryable, "fixture"));
        }
        if let Some(t) = token {
            assert_eq!(t.account_id, self.account);
            assert_eq!(t.query, *q);
            assert_eq!(t.folder, folder);
        }
        // Deliberately return substring false positives too: the adapter must
        // inspect actual header edges, not trust substring SEARCH results.
        let candidates: Vec<_> = self.rows.iter().filter(|(f, _, _)| f == folder).collect();
        let index = token.map(|t| t.before_uid as usize).unwrap_or(0);
        let rows = candidates
            .get(index)
            .map(|(_, headers, uid)| {
                let reference = p::MessageRef {
                    graph_id: None,
                    account_id: self.account.clone(),
                    folder: folder.into(),
                    uidvalidity: 1,
                    uid: *uid,
                };
                p::Metadata {
                    identity: if self.gmail {
                        p::Identity::Gmail {
                            account_id: self.account.clone(),
                            message_id: *uid as u64,
                        }
                    } else {
                        p::Identity::Imap(reference.clone())
                    },
                    reference,
                    gmail_thread_id: self.gmail.then_some(if *uid < 3 { 123 } else { 456 }),
                    labels: vec![],
                    seen: false,
                    starred: false,
                    size: headers.len() as u32,
                    internal_date: "29-Oct-2026 00:00:00 +0000".into(),
                    headers: headers.clone(),
                }
            })
            .into_iter()
            .collect();
        Ok(p::Page {
            rows,
            next: (index + 1 < candidates.len()).then(|| p::PageToken {
                graph_next: None,
                modseq: None,
                account_id: self.account.clone(),
                folder: folder.into(),
                uidvalidity: 1,
                upper_uid: 100,
                before_uid: (index + 1) as u32,
                query: q.clone(),
            }),
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
        panic!("thread discovery must not fetch message bodies")
    }
    async fn mutate(&self, _: &p::MessageRef, _: p::Mutation) -> p::Result<p::MutationResult> {
        panic!("thread discovery must not mutate mail")
    }
    async fn wait_for_change(&self, _: &str, _: Duration) -> p::Result<bool> {
        unreachable!()
    }
    async fn submit(
        &self,
        _: &p::mime::PreparedMessage,
        _: &dyn p::SubmissionGate,
    ) -> p::SubmissionOutcome {
        panic!("must not send")
    }
    async fn reconcile_sent(&self, _: &str, _: &[u8]) -> p::Result<p::SentOutcome> {
        panic!("must not file")
    }
}
async fn account(store: &Store, user: &str) -> AuthorizedAccount {
    let input: ConnectAccount = serde_json::from_value(serde_json::json!({"email":"owner@example.invalid","display_name":"Fixture","provider":"imap","imap_host":"imap.example.invalid","imap_port":993,"imap_tls":"implicit","smtp_host":"smtp.example.invalid","smtp_port":465,"smtp_tls":"implicit","username":"fixture","password":"synthetic-only"})).unwrap();
    let account = store.connect(user, &input).await.unwrap();
    store.authorize(user, &account.id).await.unwrap()
}
async fn finish(
    runtime: &MailRuntime,
    store: &Store,
    a: &AuthorizedAccount,
    seed: &str,
    mut page: ConversationPage,
) -> ConversationPage {
    for _ in 0..100 {
        let Some(token) = page.continuation else {
            assert!(page.complete);
            return page;
        };
        page = runtime
            .conversation(store, a, seed, Some(&token))
            .await
            .unwrap();
    }
    panic!("continuation must terminate")
}
#[test]
fn headers_handle_single_repeated_missing_malformed_and_case_safely() {
    let h = parse(&raw("Root", "References: <ancestor@EXAMPLE.INVALID>\r\nReferences: <other@example.invalid>\r\nIn-Reply-To: <parent@example.invalid>\r\n", 28), None).unwrap();
    assert_eq!(
        h.ids,
        vec![
            "Root@example.invalid",
            "ancestor@example.invalid",
            "other@example.invalid",
            "parent@example.invalid"
        ]
    );
    assert_ne!(
        canonical("Root@example.invalid"),
        canonical("root@example.invalid")
    );
    assert!(canonical("evil\r\n@x").is_none());
    assert!(parse(b"Subject: same\r\n\r\n", None)
        .unwrap()
        .ids
        .is_empty());
    assert!(canonical("no-id").is_none());
    let repeated = raw("root", "Message-ID: <root@EXAMPLE.INVALID>\r\n", 28);
    let parsed = mail_parser::MessageParser::default()
        .parse(&repeated)
        .unwrap();
    assert_eq!(message_id(&parsed).as_deref(), Some("root@example.invalid"));
    let conflicting = raw("root", "Message-ID: <unrelated@example.invalid>\r\nReferences: <parent@example.invalid>\r\nReferences: <older@example.invalid>\r\n", 28);
    let parsed = mail_parser::MessageParser::default()
        .parse(&conflicting)
        .unwrap();
    assert!(message_id(&parsed).is_none());
    assert_eq!(
        references(&parsed),
        vec!["parent@example.invalid", "older@example.invalid"]
    );
    assert_eq!(
        parse(&conflicting, None).unwrap().ids,
        vec!["older@example.invalid", "parent@example.invalid"]
    );
}
#[test]
fn missing_headers_remain_singletons_and_limits_never_claim_completeness() {
    let message: Message = serde_json::from_value(serde_json::json!({"id":"fixture","account_id":"account","thread_id":"same-subject-is-not-a-key","provider_id":"fixture","folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},"to":[],"subject":"Floor heating","preview":"","received_at":"2026-10-26T10:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    let mut state = Discovery {
        account: "account".into(),
        seed: "fixture".into(),
        created: Instant::now(),
        folders: vec!["INBOX".into(), "Sent".into()],
        queue: VecDeque::new(),
        searched: HashSet::new(),
        rows: HashMap::new(),
        limited: false,
        coverage_limited: false,
        reconcile_pending: false,
        observed: HashSet::new(),
        revision: 0,
        catch_up: false,
        finished: None,
    };
    state.add(message.clone(), Headers::default());
    assert!(state.queue.is_empty());
    assert!(state.page("token".into()).complete);
    assert!(!state.accepts(&Headers::default()));
    state.coverage_limited = true;
    assert!(!state.page("token".into()).complete);
    state.rows.clear();
    state.coverage_limited = false;
    let (_, document) = remembered_document("fixture", &raw("root", "", 28), None, 0).unwrap();
    let mut headers: Headers = serde_json::from_str(&document).unwrap();
    assert!(headers.size.is_none(), "Graph does not supply a MIME size in message listings");
    headers.size = Some(200);
    for id in ["one", "two"] {
        let mut graph = message.clone();
        graph.id = id.into();
        graph.provider_id = format!("graph:{id}");
        state.rows.insert(id.into(), (graph, headers.clone()));
    }
    assert_eq!(state.page("token".into()).rows.len(), 2, "distinct Graph messages must not collapse");
    state.rows.clear();
    state.add(
        message,
        Headers {
            ids: (0..300).map(|i| format!("{i}@example.invalid")).collect(),
            ..Default::default()
        },
    );
    let page = state.page("token".into());
    assert!(!page.complete);
    assert!(page.continuation.is_none());
    assert_eq!(page.rows.len(), 1);
    assert!(state.searched.len() <= 256);
    assert!(state.queue.len() <= 8192);
    let query = p::Search {
        conversation_id: Some("root@example.invalid".into()),
        ..Default::default()
    }
    .imap_query()
    .unwrap();
    assert!(query.contains("HEADER References"));
    assert!(!query.contains("TEXT"));
    assert!(p::Search {
        conversation_id: Some("bad\r\nLOGOUT".into()),
        ..Default::default()
    }
    .imap_query()
    .is_err());
}
#[tokio::test]
async fn incoming_sent_transitive_out_of_order_copies_collision_pagination_failure_and_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [19; 32]).await.unwrap();
    let user = store.user("fixture", "owner").await.unwrap();
    let a = account(&store, &user).await;
    let b = account(&store, &user).await;
    let runtime = MailRuntime::default();
    let root = raw("root", "", 26);
    let provider = Arc::new(Provider {
        account: a.id().into(),
        gmail: false,
        delay_ms: 0,
        fail: AtomicBool::new(false),
        scans: AtomicUsize::new(0),
        rows: vec![
            ("INBOX".into(), root.clone(), 1),
            ("INBOX".into(), raw("unrelated", "", 25), 2),
            ("INBOX".into(), raw("prefix-root", "", 25), 3),
            (
                "INBOX".into(),
                b"From: nobody@example.invalid\r\nSubject: Floor heating\r\n\r\n".to_vec(),
                4,
            ),
            (
                "Sent Items".into(),
                raw("reply", "In-Reply-To: <root@example.invalid>\r\n", 27),
                5,
            ),
            (
                "Archive".into(),
                raw("last", "References: <reply@example.invalid>\r\n", 28),
                6,
            ),
            ("Archive".into(), root, 7), // identical mailbox copy, one visible item
            (
                "Archive".into(),
                raw("root", "X-Fixture: distinct-message-id-collision\r\n", 29),
                8,
            ),
        ],
    });
    runtime
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider.clone());
    let seed = runtime
        .page(&store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap()
        .rows[0]
        .id
        .clone();
    let first = runtime.conversation(&store, &a, &seed, None).await.unwrap();
    let token = first
        .continuation
        .clone()
        .expect("request budget must paginate");
    // A cursor is bound to BOTH account and seed, regardless of shared RFC IDs.
    let other: Message = serde_json::from_value(serde_json::json!({"id":seed,"account_id":b.id(),"thread_id":"same","provider_id":"same","folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},"to":[],"subject":"Floor heating","preview":"","received_at":"2026-10-26T10:00:00Z","unread":true,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap();
    store.upsert_message(&b, other).await.unwrap();
    assert_eq!(
        runtime
            .conversation(&store, &b, &seed, Some(&token))
            .await
            .unwrap_err()
            .0,
        StatusCode::NOT_FOUND
    );
    provider.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        runtime
            .conversation(&store, &a, &seed, Some(&token))
            .await
            .unwrap_err()
            .0,
        StatusCode::BAD_GATEWAY
    );
    let page = finish(&runtime, &store, &a, &seed, first).await;
    assert_eq!(page.rows.len(), 4); // root, reply, last, distinct duplicate-ID message
    assert!(page.rows.iter().all(|m| m.account_id == a.id()));
    assert_eq!(
        page.rows
            .iter()
            .map(|m| &m.received_at[..10])
            .collect::<Vec<_>>(),
        vec!["2026-10-26", "2026-10-27", "2026-10-28", "2026-10-29"]
    );
    assert!(page
        .rows
        .iter()
        .any(|m| m.folder_ids.contains(&"Sent Items".into())));
    assert!(provider.scans.load(Ordering::SeqCst) < 100);
    // Unknown and expired continuations never restart unbounded provider work.
    assert!(runtime
        .conversation(&store, &a, &seed, Some("unknown"))
        .await
        .is_err());
}
#[tokio::test]
async fn gmail_native_thread_without_rfc_headers_excludes_other_provider_threads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path(), [20; 32]).await.unwrap();
    let user = store.user("fixture", "gmail").await.unwrap();
    let a = account(&store, &user).await;
    let runtime = MailRuntime::default();
    let provider = Arc::new(Provider {
        account: a.id().into(),
        gmail: true,
        delay_ms: 0,
        fail: AtomicBool::new(false),
        scans: AtomicUsize::new(0),
        rows: vec![
            (
                "INBOX".into(),
                b"From: friend@example.invalid\r\nSubject: Same\r\n\r\n".to_vec(),
                1,
            ),
            (
                "Sent Items".into(),
                b"From: owner@example.invalid\r\nSubject: Same\r\n\r\n".to_vec(),
                2,
            ),
            (
                "Archive".into(),
                b"From: other@example.invalid\r\nSubject: Same\r\n\r\n".to_vec(),
                3,
            ),
        ],
    });
    runtime
        .providers
        .lock()
        .await
        .insert(a.id().into(), provider);
    let seed = runtime
        .page(&store, &a, "INBOX", p::Search::default(), None)
        .await
        .unwrap()
        .rows[0]
        .id
        .clone();
    let first = runtime.conversation(&store, &a, &seed, None).await.unwrap();
    assert_eq!(
        finish(&runtime, &store, &a, &seed, first).await.rows.len(),
        2
    );
}
