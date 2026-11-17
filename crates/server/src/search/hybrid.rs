//! Frozen fusion prefix followed by the lexical stream. Source validation is
//! repeated when presenting a saved page; database and provider state stay live.
use super::*;
use dakia_storage::{CachedSearchFilter, SemanticMatch};
use sha2::{Digest, Sha256};
pub(super) const LEXICAL_HEAD: usize = 512;
const SEMANTIC_HEAD: usize = 200;
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct FrozenHit {
    account: String,
    id: String,
    source: String,
    headers: String,
    date: String,
    pub score: f64,
    excerpt: Option<String>,
    attachment: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct FrozenPage {
    pub remaining: Vec<FrozenHit>,
    pub coverage: Vec<AccountCoverage>,
    pub complete: bool,
    pub degradation: Vec<SearchDegradation>,
    pub lexical_rank: usize,
}
fn headers(message: &Message) -> String {
    let text = serde_json::to_vec(&(&message.sender, &message.to, &message.subject))
        .expect("serializable message headers");
    format!("{:x}", Sha256::digest(text))
}
impl FrozenHit {
    fn new(message: &Message, score: f64) -> Self {
        Self {
            account: message.account_id.clone(),
            id: message.id.clone(),
            source: message.provider_id.clone(),
            headers: headers(message),
            date: message.received_at.clone(),
            score,
            excerpt: None,
            attachment: None,
        }
    }
    pub fn details(&self) -> SearchMatch {
        SearchMatch {
            account_id: self.account.clone(),
            message_id: self.id.clone(),
            score: self.score,
            excerpt: self.excerpt.clone(),
            attachment: self.attachment.clone(),
        }
    }
}
pub(super) fn fuse(lexical: &[(Message, f64)], mut semantic: Vec<SemanticMatch>) -> Vec<FrozenHit> {
    let mut hits = BTreeMap::new();
    for (rank, (message, _)) in lexical.iter().enumerate() {
        hits.insert(
            (message.account_id.clone(), message.id.clone()),
            FrozenHit::new(message, 1. / (61. + rank as f64)),
        );
    }
    semantic.sort_by(|a, b| {
        a.distance
            .total_cmp(&b.distance)
            .then_with(|| b.message.received_at.cmp(&a.message.received_at))
            .then_with(|| a.message.account_id.cmp(&b.message.account_id))
            .then_with(|| b.message.id.cmp(&a.message.id))
    });
    let mut seen = BTreeSet::new();
    for (rank, hit) in semantic
        .into_iter()
        .filter(|m| seen.insert((m.message.account_id.clone(), m.message.id.clone())))
        .take(SEMANTIC_HEAD)
        .enumerate()
    {
        let score = 1. / (61. + rank as f64);
        let entry = hits
            .entry((hit.message.account_id.clone(), hit.message.id.clone()))
            .or_insert_with(|| FrozenHit::new(&hit.message, 0.));
        entry.score += score;
        entry.excerpt = Some(hit.excerpt.chars().take(320).collect());
        entry.attachment = hit.attachment;
    }
    let mut hits: Vec<_> = hits.into_values().collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.date.cmp(&a.date))
            .then_with(|| a.account.cmp(&b.account))
            .then_with(|| b.id.cmp(&a.id))
    });
    hits
}
pub(super) async fn frozen(
    app: &App,
    user: &Identity,
    r: &SearchRequest,
    scopes: &[dakia_storage::AuthorizedAccount],
    query: &Search,
    wanted: Option<&str>,
    cursor: &mut Cursor,
) -> Result<Option<SearchPage>> {
    let Some(mut frozen) = cursor.fused.take() else {
        return Ok(None);
    };
    if frozen.remaining.is_empty() {
        cursor.fused = Some(frozen);
        return Ok(None);
    }
    let mut rows = vec![];
    let mut matches = vec![];
    while !frozen.remaining.is_empty() && rows.len() < 50 {
        let take = (50 - rows.len()).min(frozen.remaining.len());
        let batch: Vec<_> = frozen.remaining.drain(..take).collect();
        let mut current = BTreeMap::new();
        for a in scopes {
            let ids: Vec<_> = batch
                .iter()
                .filter(|h| h.account == a.id())
                .map(|h| h.id.clone())
                .collect();
            if ids.is_empty() {
                continue;
            }
            let folder = frozen
                .coverage
                .iter()
                .find(|c| c.account_id == a.id())
                .map(|c| c.folder_ids.as_slice());
            let plan =
                dakia_domain::mailbox_query::MailboxQuery::parse(&r.q).map_err(|_| invalid())?;
            let (since, before) = plan.date_bounds().map_err(|_| invalid())?;
            let filter = CachedSearchFilter {
                classification: r.classification(),
                since: since.as_deref(),
                before: before.as_deref(),
                from: query.from.as_deref(),
                to: query.to.as_deref(),
                subject: query.subject.as_deref(),
                seen: query.seen,
                starred: query.starred,
                has_attachment: plan.has_attachment,
                folders: if wanted.is_some() { folder } else { None },
                category: r.category.as_deref(),
                snoozed_only: wants_snoozed(&r.q),
                hide_snoozed: wanted.is_some_and(|f| f.eq_ignore_ascii_case("INBOX")),
                ..Default::default()
            };
            let messages =
                match app
                    .store
                    .current_indexed_search_rows(a, &ids, &filter)
                    .await
                {
                    Ok(messages) => messages,
                    Err(_) => {
                        frozen.complete = false;
                        if let Some(status) =
                            frozen.coverage.iter_mut().find(|c| c.account_id == a.id())
                        {
                            status.state = CoverageState::Error;
                            status.error = Some("account_search_unavailable".into());
                        }
                        if !frozen.degradation.iter().any(|d| {
                            d.account_id == a.id() && d.code == "account_search_unavailable"
                        }) {
                            frozen.degradation.push(SearchDegradation {
                                account_id: a.id().into(),
                                code: "account_search_unavailable".into(),
                            });
                        }
                        continue;
                    }
                };
            for message in messages {
                current.insert((a.id().to_owned(), message.id.clone()), message);
            }
        }
        for hit in batch {
            let Some(message) = current.remove(&(hit.account.clone(), hit.id.clone())) else {
                continue;
            };
            let key = format!("{}:{}", hit.account, hit.id);
            if message.provider_id != hit.source
                || headers(&message) != hit.headers
                || !cursor.seen.insert(key)
            {
                continue;
            }
            matches.push(hit.details());
            rows.push(message);
        }
    }
    let complete = frozen.complete;
    let coverage = frozen.coverage.clone();
    let degradation = frozen.degradation.clone();
    let more = !frozen.remaining.is_empty() || cursor.fused_lexical_more;
    cursor.local_index = more;
    cursor.fused = Some(frozen);
    let continuation = if more || !complete {
        Some(
            app.store
                .save_search_cursor(
                    &user.user,
                    &serde_json::to_string(cursor).map_err(StorageError::from)?,
                )
                .await?,
        )
    } else {
        None
    };
    for row in &mut rows {
        if let Some(a) = scopes.iter().find(|a| a.id() == row.account_id) {
            *row = app.store.visible_message(a, row.clone()).await?;
        }
    }
    let total = rows.len() as u32;
    let classified = rows
        .iter()
        .filter(|m| m.category_status == CategoryStatus::Ready)
        .count() as u32;
    Ok(Some(SearchPage {
        rows,
        continuation,
        coverage,
        classification: ClassificationCoverage { classified, total },
        pending: !complete,
        counts_exact: false,
        cache_preview: (!complete).then_some(true),
        index_page: more.then_some(true),
        supplement: None,
        search: Some(SearchMetadata {
            ranked: true,
            matches,
            degradation,
        }),
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: &str, date: &str) -> Message {
        serde_json::from_value(serde_json::json!({"id":id,"account_id":"a","thread_id":id,"provider_id":id,"folder_ids":["INBOX"],"sender":{"email":"sender@example.invalid","name":null},"to":[],"subject":id,"preview":"","received_at":date,"unread":false,"starred":false,"category":null,"category_status":"pending","revision":0})).unwrap()
    }
    #[test]
    fn reciprocal_rank_fusion_is_deduplicated_and_uses_shared_ties() {
        let old = message("old", "2020");
        let new = message("new", "2026");
        let only = message("semantic", "2021");
        let vector = |m: Message, distance| SemanticMatch {
            attachment: (m.id == "semantic").then(|| "report.pdf".into()),
            message: m,
            distance,
            excerpt: "matching passage".into(),
            field: 1,
        };
        let fused = fuse(
            &[(old.clone(), -5.), (new.clone(), -1.)],
            vec![
                vector(new, 0.1),
                vector(only.clone(), 0.2),
                vector(only, 0.3),
            ],
        );
        assert_eq!(fused.len(), 3);
        let attachment = fused.iter().find(|hit| hit.id == "semantic").unwrap();
        assert_eq!(
            attachment.details().attachment.as_deref(),
            Some("report.pdf")
        );
        let restored: FrozenHit =
            serde_json::from_str(&serde_json::to_string(attachment).unwrap()).unwrap();
        assert_eq!(
            restored.details().attachment,
            attachment.details().attachment
        );
        assert_eq!(fused[0].id, "new");
        assert!(fused[0].score > fused[1].score);
        assert!(fused
            .iter()
            .any(|m| m.id == "semantic" && m.excerpt.as_deref() == Some("matching passage")));
        assert_eq!(fuse(&[(old, -1.)], vec![])[0].score, 1. / 61.);
    }
}
