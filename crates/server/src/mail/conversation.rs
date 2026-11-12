//! On-demand, account-local header graph discovery across every selectable folder.
//! No subject guessing, body searches, mailbox writes, or globally shared RFC IDs.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::time::Instant;
#[cfg(test)]
mod tests;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Headers {
    ids: Vec<String>,
    gmail: Option<u64>,
    fingerprint: String,
    #[serde(default)]
    size: Option<u32>,
    date: Option<String>,
}
fn canonical(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('<').trim_end_matches('>');
    let (local, domain) = value.rsplit_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || value.len() > 998
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>\"\\".contains(c))
    {
        return None;
    }
    Some(format!("{local}@{}", domain.to_ascii_lowercase()))
}
fn header_ids(parsed: &mail_parser::Message<'_>, name: mail_parser::HeaderName<'_>) -> Vec<String> {
    let mut ids = vec![];
    for header in parsed.headers().iter().filter(|h| h.name == name) {
        let values: Vec<&str> = if let Some(list) = header.value.as_text_list() {
            list.iter().map(|v| v.as_ref()).collect()
        } else {
            header.value.as_text().into_iter().collect()
        };
        for value in values {
            if let Some(id) = canonical(value) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
    }
    ids
}
pub(super) fn message_id(parsed: &mail_parser::Message<'_>) -> Option<String> {
    let mut ids = header_ids(parsed, mail_parser::HeaderName::MessageId);
    // Repeated identical headers are harmless. Conflicting Message-IDs must
    // not bridge unrelated conversations or manufacture reply ancestry.
    (ids.len() == 1).then(|| ids.remove(0))
}
pub(super) fn references(parsed: &mail_parser::Message<'_>) -> Vec<String> {
    let ids = header_ids(parsed, mail_parser::HeaderName::References);
    if ids.is_empty() {
        header_ids(parsed, mail_parser::HeaderName::InReplyTo)
    } else {
        ids
    }
}
fn parse(raw: &[u8], gmail: Option<u64>) -> Result<Headers> {
    let parsed = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or(ApiFailure(
            StatusCode::BAD_GATEWAY,
            "invalid_message_headers",
        ))?;
    let mut ids: Vec<String> = message_id(&parsed).into_iter().collect();
    ids.extend(references(&parsed));
    ids.extend(header_ids(&parsed, mail_parser::HeaderName::InReplyTo));
    ids.sort();
    ids.dedup();

    // Header fingerprints plus provider size distinguish mailbox copies from
    // ordinary Message-ID collisions, without reading unrelated bodies.
    let end = raw
        .windows(4)
        .position(|v| v == b"\r\n\r\n")
        .unwrap_or(raw.len());
    let fingerprint = format!("{:x}", Sha256::digest(&raw[..end]));
    let date = parsed
        .date()
        .and_then(|d| chrono::DateTime::from_timestamp(d.to_timestamp(), 0))
        .map(|d| d.to_rfc3339());
    Ok(Headers {
        ids,
        gmail,
        fingerprint,
        size: None,
        date,
    })
}
pub(super) async fn remember(
    store: &Store,
    a: &AuthorizedAccount,
    id: &str,
    raw: &[u8],
    gmail: Option<u64>,
    size: u32,
) -> Result<()> {
    let (key, document) = remembered_document(id, raw, gmail, size)?;
    store.put_provider_document(a, &key, &document).await?;
    Ok(())
}

pub(super) fn remembered_document(
    id: &str,
    raw: &[u8],
    gmail: Option<u64>,
    size: u32,
) -> Result<(String, String)> {
    let mut headers = parse(raw, gmail)?;
    headers.size = (size > 0).then_some(size);
    Ok((
        format!("thread-headers:{id}"),
        serde_json::to_string(&headers).map_err(Error::from)?,
    ))
}
async fn headers(
    runtime: &MailRuntime,
    store: &Store,
    a: &AuthorizedAccount,
    m: &Message,
) -> Result<Headers> {
    if let Some(raw) = store
        .provider_document(a, &format!("thread-headers:{}", m.id))
        .await?
    {
        return serde_json::from_str(&raw)
            .map_err(Error::from)
            .map_err(Into::into);
    }
    // Upgrade only the explicitly selected legacy message. Other discoveries use
    // BODY.PEEK[HEADER] metadata, never fetch bodies to build a conversation.
    let raw = runtime.raw(store, a, &m.id).await?;
    let gmail = m
        .thread_id
        .strip_prefix("gmail-thread:")
        .and_then(|v| v.parse().ok());
    remember(store, a, &m.id, &raw, gmail, raw.len() as u32).await?;
    let mut h = parse(&raw, gmail)?;
    h.size = Some(raw.len() as u32);
    Ok(h)
}
#[derive(Clone)]
struct Work {
    folder: String,
    query: p::Search,
    continuation: Option<String>,
}
pub(super) struct Discovery {
    account: String,
    seed: String,
    created: Instant,
    folders: Vec<String>,
    queue: VecDeque<Work>,
    searched: HashSet<String>,
    rows: HashMap<String, (Message, Headers)>,
    limited: bool,
    coverage_limited: bool,
    reconcile_pending: bool,
    observed: HashSet<String>,
    revision: i64,
    catch_up: bool,
    finished: Option<Instant>,
}
impl Discovery {
    fn component(&self) -> String {
        let mut keys: Vec<_> = self
            .rows
            .values()
            .flat_map(|(_, h)| {
                h.ids
                    .iter()
                    .map(|id| format!("rfc:{id}"))
                    .chain(h.gmail.map(|id| format!("gmail:{id}")))
            })
            .collect();
        if keys.is_empty() {
            keys.push(format!("seed:{}", self.seed));
        }
        keys.sort();
        keys.dedup();
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&keys).expect("string vector serializes"))
        )
    }
    fn add(&mut self, message: Message, h: Headers) {
        if self.rows.len() >= 1000 && !self.rows.contains_key(&message.id) {
            self.limited = true;
            return;
        }
        let keys: Vec<(String, p::Search)> = if let Some(gmail) = h.gmail {
            vec![(
                format!("gmail:{gmail}"),
                p::Search {
                    gmail_thread: Some(gmail),
                    ..Default::default()
                },
            )]
        } else {
            h.ids
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        p::Search {
                            conversation_id: Some(id.clone()),
                            ..Default::default()
                        },
                    )
                })
                .collect()
        };
        for (key, query) in keys {
            if self.searched.contains(&key) {
                continue;
            }
            if self.searched.len() >= 256 || self.queue.len() + self.folders.len() > 8192 {
                self.limited = true;
                break;
            }
            if self.searched.insert(key) {
                for folder in &self.folders {
                    self.queue.push_back(Work {
                        folder: folder.clone(),
                        query: query.clone(),
                        continuation: None,
                    });
                }
            }
        }
        self.rows.insert(message.id.clone(), (message, h));
    }
    fn accepts(&self, h: &Headers) -> bool {
        self.rows
            .values()
            .any(|(_, old)| match (old.gmail, h.gmail) {
                (Some(a), Some(b)) => a == b,
                _ => old.ids.iter().any(|id| h.ids.contains(id)),
            })
    }
    fn page(&self, token: String) -> ConversationPage {
        let mut values: Vec<_> = self.rows.values().collect();
        let timestamp = |m: &Message, h: &Headers| {
            chrono::DateTime::parse_from_rfc3339(h.date.as_ref().unwrap_or(&m.received_at))
                .map(|d| d.timestamp_millis())
                .unwrap_or(0)
        };
        values.sort_by(|(a, ah), (b, bh)| {
            timestamp(a, ah)
                .cmp(&timestamp(b, bh))
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut seen = HashSet::new();
        let rows = values
            .into_iter()
            .filter(|(m, h)| {
                // Native provider IDs distinguish messages even with identical headers.
                // IMAP copies need matching RFC evidence, headers and a known size.
                seen.insert(if m.provider_id.starts_with("gmail:") || m.provider_id.starts_with("graph:") {
                    m.provider_id.clone()
                } else if h.ids.is_empty() || h.size.is_none() {
                    m.id.clone()
                } else {
                    format!("{}:{}", h.fingerprint, h.size.unwrap())
                })
            })
            .map(|(m, h)| {
                let mut m = m.clone();
                if let Some(date) = &h.date {
                    m.received_at = date.clone();
                }
                m
            })
            .collect();
        ConversationPage {
            rows,
            continuation: ((self.reconcile_pending || !self.queue.is_empty()) && !self.limited)
                .then_some(token),
            complete: !self.reconcile_pending
                && self.queue.is_empty()
                && !self.limited
                && !self.coverage_limited,
        }
    }
}
impl MailRuntime {
    #[cfg(test)]
    pub(crate) async fn conversation(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
        continuation: Option<&str>,
    ) -> Result<ConversationPage> {
        self.conversation_read(store, a, id, continuation, false, false)
            .await
    }
    pub(crate) async fn conversation_read(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        id: &str,
        continuation: Option<&str>,
        snapshot: bool,
        force: bool,
    ) -> Result<ConversationPage> {
        if snapshot && continuation.is_some() {
            return Err(ApiFailure(
                StatusCode::BAD_REQUEST,
                "invalid_conversation_query",
            ));
        }
        store.message(a, id).await?; // authorizes target before cursor lookup
        let (token, state) = if let Some(token) = continuation {
            let states = self.conversations.lock().await;
            let (account, state) = states.get(token).cloned().ok_or(ApiFailure(
                StatusCode::CONFLICT,
                "conversation_refresh_required",
            ))?;
            if account != a.id() {
                return Err(ApiFailure(StatusCode::NOT_FOUND, "not_found"));
            }
            (token.to_owned(), state)
        } else {
            let revision = store.thread_revision(a).await?;
            let seed = store.message(a, id).await?;
            let (h, missing_headers) = if snapshot {
                let cached = store
                    .provider_document(a, &format!("thread-headers:{id}"))
                    .await?;
                let missing = cached.is_none();
                (
                    cached
                        .map(|raw| serde_json::from_str(&raw).map_err(Error::from))
                        .transpose()?
                        .unwrap_or_default(),
                    missing,
                )
            } else {
                (headers(self, store, a, &seed).await?, false)
            };
            let mut state = Discovery {
                account: a.id().into(),
                seed: id.into(),
                created: Instant::now(),
                folders: Vec::new(),
                queue: VecDeque::new(),
                searched: HashSet::new(),
                rows: HashMap::new(),
                limited: false,
                coverage_limited: missing_headers,
                reconcile_pending: true,
                observed: HashSet::new(),
                revision,
                catch_up: false,
                finished: None,
            };
            state.add(store.visible_message(a, seed).await?, h);
            // Indexed, account-local known members never wait on provider I/O.
            let mut looked_up = HashSet::new();
            loop {
                let mut edges = Vec::new();
                for (_, h) in state.rows.values() {
                    let keys = h
                        .ids
                        .iter()
                        .map(|id| format!("rfc:{id}"))
                        .chain(h.gmail.map(|id| format!("gmail:{id}")));
                    for key in keys {
                        if looked_up.contains(&key) {
                            continue;
                        }
                        if looked_up.len() >= 256 {
                            state.limited = true;
                            break;
                        }
                        looked_up.insert(key.clone());
                        edges.push(key);
                    }
                }
                if edges.is_empty() || state.limited {
                    break;
                }
                let neighbors = store.thread_neighbors(a, &edges).await?;
                if neighbors.len() > 1000 {
                    state.limited = true;
                }
                for (message, h) in neighbors.into_iter().take(1000) {
                    if state.rows.contains_key(&message.id) {
                        continue;
                    }
                    let h: Headers = serde_json::from_str(&h).map_err(Error::from)?;
                    if state.accepts(&h) {
                        state.add(store.visible_message(a, message).await?, h);
                    }
                }
            }
            if !force
                && ((store.thread_coverage_fresh(a).await?
                    && store.thread_revision(a).await? == revision)
                    || store
                        .thread_verified(a, &state.component(), revision)
                        .await?)
            {
                state.reconcile_pending = false;
            }
            if snapshot {
                let mut page = state.page(String::new());
                page.continuation = None;
                return Ok(page);
            }
            // Share bounded provider work, never stale display rows. A dirty
            // recent result remains partial until the next verification window.
            if !force && state.reconcile_pending {
                let states: Vec<_> = self
                    .conversations
                    .lock()
                    .await
                    .iter()
                    .filter(|(_, (account, _))| account == a.id())
                    .map(|(token, (_, state))| (token.clone(), state.clone()))
                    .collect();
                for (token, shared) in states {
                    let old = shared.lock().await;
                    if old.account == a.id()
                        && (old.seed == id
                            || (old.rows.contains_key(id) && old.component() == state.component()))
                        && old.created.elapsed() < Duration::from_secs(900)
                        && old
                            .finished
                            .is_none_or(|at| at.elapsed() < Duration::from_secs(30))
                        && (old.reconcile_pending || !old.folders.is_empty())
                    {
                        let mut page = state.page(token.clone());
                        if !old.reconcile_pending && old.queue.is_empty() {
                            page.continuation = None;
                        }
                        return Ok(page);
                    }
                }
            }
            if !state.reconcile_pending {
                return Ok(state.page(String::new()));
            }
            let state = Arc::new(Mutex::new(state));
            let token = uuid::Uuid::new_v4().to_string();
            let mut states = self.conversations.lock().await;
            states.retain(|_, (_, state)| {
                state
                    .try_lock()
                    .map(|s| s.created.elapsed() < Duration::from_secs(900))
                    .unwrap_or(true)
            });
            if states.len() >= 128 {
                return Err(ApiFailure(
                    StatusCode::TOO_MANY_REQUESTS,
                    "conversation_busy",
                ));
            }
            states.insert(token.clone(), (a.id().into(), state.clone()));
            return Ok(state.lock().await.page(token));
        };
        let mut state = state.lock().await;
        if state.account != a.id() || (state.seed != id && !state.rows.contains_key(id)) {
            return Err(ApiFailure(StatusCode::NOT_FOUND, "not_found"));
        }
        if state.created.elapsed() >= Duration::from_secs(900) {
            return Err(ApiFailure(
                StatusCode::CONFLICT,
                "conversation_refresh_required",
            ));
        }
        if state.finished.is_some() {
            // Replaying a terminal cursor must not renew an old absence proof.
            drop(state);
            return Box::pin(self.conversation_read(store, a, id, None, true, false)).await;
        }
        if state.reconcile_pending {
            state.folders = self
                .folders(store, a)
                .await?
                .into_iter()
                .map(|f| f.name)
                .collect();
            state.searched.clear();
            let rows = std::mem::take(&mut state.rows);
            for (_, (message, h)) in rows {
                state.add(message, h);
            }
            state.reconcile_pending = false;
            state.revision = store.thread_revision(a).await?;
        }
        // Bound each request's work. The opaque account/seed-bound continuation
        // preserves every folder/page/frontier, including out-of-order ancestors.
        for _ in 0..8 {
            if state.limited {
                break;
            }
            let Some(work) = state.queue.front().cloned() else {
                break;
            };
            let page = self
                .page(
                    store,
                    a,
                    &work.folder,
                    work.query.clone(),
                    work.continuation.as_deref(),
                )
                .await?;
            state.coverage_limited |= page
                .coverage
                .iter()
                .any(|c| c.state != CoverageState::Complete);
            for message in page.rows {
                let h = headers(self, store, a, &message).await?;
                // IMAP HEADER is substring matching. Verify exact RFC graph edges.
                if state.accepts(&h) {
                    state.observed.insert(message.id.clone());
                    state.add(message, h);
                }
            }
            // Advance only after every row's evidence has been accepted. A
            // failure can retry the same page without losing or duplicating it.
            state.queue.pop_front();
            if let Some(next) = page.continuation {
                state.queue.push_front(Work {
                    continuation: Some(next),
                    ..work
                });
            }
        }
        if state.queue.is_empty() && !state.searched.is_empty() && !state.limited {
            // Cached membership is provisional until reconciliation observes it.
            // Never certify an old cached UID simply because its header survived.
            let observed = state.observed.clone();
            let seed = state.seed.clone();
            let before = state.rows.len();
            state
                .rows
                .retain(|id, _| id == &seed || observed.contains(id));
            state.coverage_limited |= before != state.rows.len() || !observed.contains(&seed);
        }
        // Changes after an already-scanned frontier require a final catch-up.
        // Continuous churn is bounded to two passes and remains honest partial.
        if state.queue.is_empty()
            && !state.searched.is_empty()
            && !state.limited
            && store.thread_revision(a).await? != state.revision
        {
            if !state.catch_up {
                state.catch_up = true;
                state.reconcile_pending = true;
                state.observed.clear();
            } else {
                state.coverage_limited = true;
            }
        }
        let mut page = state.page(token);
        if page.continuation.is_none() {
            state.finished.get_or_insert_with(Instant::now);
        }
        if page.complete {
            page.complete = store
                .verify_thread_revision(a, &state.component(), state.revision)
                .await?;
        }
        Ok(page)
    }
}
