use super::*;
use dakia_mail_provider::Search;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
mod hybrid;
mod indexed;
mod preview;
// Version 8 treats attachment presence as a strict filter on every retrieval path.
const SEARCH_CURSOR_VERSION: u32 = 8;

async fn indexed_or_provider(
    app: &App,
    user: &Identity,
    request: &SearchRequest,
    scopes: &[dakia_storage::AuthorizedAccount],
    query: &Search,
    has_attachment: bool,
    wanted: Option<&str>,
    cursor: &mut Cursor,
) -> Option<SearchPage> {
    let checkpoint = cursor.clone();
    let mut page = match indexed::page(
        app,
        user,
        request,
        scopes,
        query,
        has_attachment,
        wanted,
        cursor,
    )
    .await
    {
        Ok(page) => page,
        Err(_) => {
            *cursor = checkpoint;
            tracing::warn!(target:"dakia_server::diagnostics",operation="search_index",outcome="unavailable","using provider search");
            None
        }
    };
    if query.text.is_some()
        && !request.suggest
        && page.as_ref().is_none_or(|p| p.rows.is_empty() && p.pending)
    {
        if let Some(mut cached) = preview::cached(
            app,
            user,
            request,
            scopes,
            query,
            has_attachment,
            wanted,
            cursor,
        )
        .await
        .ok()
        .flatten()
        {
            if let Some(indexed) = &page {
                cached.coverage = indexed.coverage.clone();
                cached.search = indexed.search.clone();
            }
            page = Some(cached);
        }
    }
    if page.is_none() && query.text.is_some() && !request.suggest {
        cursor.local_index = false;
        let continuation = app
            .store
            .save_search_cursor(&user.user, &serde_json::to_string(cursor).ok()?)
            .await
            .ok()?;
        page = Some(SearchPage {
            rows: vec![],
            continuation: Some(continuation),
            coverage: scopes
                .iter()
                .map(|a| AccountCoverage {
                    account_id: a.id().into(),
                    folder_ids: vec![],
                    inbox: None,
                    state: CoverageState::Pending,
                    error: None,
                })
                .collect(),
            classification: ClassificationCoverage {
                classified: 0,
                total: 0,
            },
            pending: true,
            counts_exact: false,
            cache_preview: None,
            index_page: None,
            search: Some(SearchMetadata {
                ranked: true,
                matches: vec![],
                degradation: scopes
                    .iter()
                    .map(|a| SearchDegradation {
                        account_id: a.id().into(),
                        code: "provider_search_pending".into(),
                    })
                    .collect(),
            }),
            supplement: None,
        });
    }
    let mut page = page?;
    if query.text.is_some() && page.pending && !request.suggest {
        let mut supplements = vec![];
        for status in &page.coverage {
            if status.state == CoverageState::Complete
                || cursor.provider_started.contains(&status.account_id)
            {
                continue;
            }
            let mut provider = cursor.clone();
            provider.provider_only = true;
            provider.provider_accounts = BTreeSet::from([status.account_id.clone()]);
            provider.local_index = false;
            provider.fused = None;
            let token = app
                .store
                .save_search_cursor(&user.user, &serde_json::to_string(&provider).ok()?)
                .await
                .ok()?;
            cursor.provider_started.insert(status.account_id.clone());
            supplements.push(SearchSupplement {
                account_id: status.account_id.clone(),
                continuation: token,
            });
        }
        if !supplements.is_empty() {
            page.supplement = Some(supplements);

            if page.continuation.is_some() {
                page.continuation = Some(
                    app.store
                        .save_search_cursor(&user.user, &serde_json::to_string(cursor).ok()?)
                        .await
                        .ok()?,
                );
            }
        }
    }
    Some(page)
}

#[derive(Default, Deserialize)]
pub(super) struct SearchRequest {
    #[serde(default)]
    q: String,
    accounts: Option<String>,
    continuation: Option<String>,
    category: Option<String>,
    inbox_kind: Option<InboxKind>,
    email_type: Option<DetectedType>,
    #[serde(default)]
    attention: bool,
    #[serde(default)]
    transactions: bool,
    #[serde(default)]
    exclude_trash: bool,
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    cache_first: bool,
    #[serde(default)]
    suggest: bool,
}
impl SearchRequest {
    fn classification(&self) -> ClassificationFilter {
        ClassificationFilter {
            inbox_kind: self.inbox_kind.clone(),
            email_type: self.email_type.clone(),
            attention: self.attention,
            transactions: self.transactions,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Cursor {
    #[serde(default)]
    spelling: BTreeMap<String, Vec<Vec<String>>>,
    #[serde(default)]
    provider_only: bool,
    #[serde(default)]
    provider_rank: usize,
    #[serde(default)]
    provider_accounts: BTreeSet<String>,
    #[serde(default)]
    provider_started: BTreeSet<String>,
    #[serde(default)]
    fused: Option<hybrid::FrozenPage>,
    #[serde(default)]
    fused_lexical_more: bool,
    #[serde(default)]
    version: u32,
    #[serde(default)]
    local_index: bool,
    #[serde(default)]
    index_after: BTreeMap<String, (String, String)>,
    #[serde(default)]
    index_rank_after: BTreeMap<String, (f64, String, String)>,
    #[serde(default)]
    index_done: BTreeSet<String>,
    #[serde(default)]
    exclude_trash: bool,
    #[serde(default)]
    local_inbox: bool,
    #[serde(default)]
    local_sent: bool,
    #[serde(default)]
    local_starred: bool,
    #[serde(default)]
    local_snoozed: bool,
    q: String,
    category: Option<String>,
    #[serde(default)]
    classification: ClassificationFilter,
    accounts: Vec<String>,
    folders: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    excluded_folders: BTreeMap<String, Vec<String>>,
    positions: BTreeMap<String, usize>,
    pages: BTreeMap<String, String>,
    full_coverage: BTreeMap<String, bool>,
    seen: BTreeSet<String>,
}
fn invalid() -> ApiFailure {
    ApiFailure(StatusCode::BAD_REQUEST, "invalid_search")
}
#[cfg(test)]
pub(super) fn index_units(q: &str) -> Vec<String> {
    dakia_domain::mailbox_query::MailboxQuery::parse(q)
        .map(|q| q.text_units())
        .unwrap_or_default()
}
fn wants_snoozed(q: &str) -> bool {
    dakia_domain::mailbox_query::MailboxQuery::parse(q).is_ok_and(|q| q.snoozed)
}
fn parse(q: &str) -> Result<(Search, Option<String>, bool)> {
    let plan = dakia_domain::mailbox_query::MailboxQuery::parse(q).map_err(|_| invalid())?;
    let text = plan.semantic_text();
    let text_units = plan.text_units();
    let query = Search {
        text_units,
        text: (!text.is_empty()).then_some(text),
        from: plan.from,
        to: plan.to,
        subject: plan.subject,
        since: plan.since,
        before: plan.before,
        seen: plan.seen,
        starred: plan.starred,
        ..Default::default()
    };
    query.imap_query().map_err(|_| invalid())?;
    Ok((query, plan.folder, plan.has_attachment))
}
fn select_folders(
    folders: Vec<dakia_mail_provider::Folder>,
    wanted: Option<&str>,
) -> Result<Vec<String>> {
    let mut selected = vec![];
    for folder in folders {
        if wanted.is_none_or(|w| {
            folder.name == w
                || (w.eq_ignore_ascii_case("INBOX") && folder.name.eq_ignore_ascii_case("INBOX"))
                || folder
                    .attributes
                    .iter()
                    .any(|a| a.trim_start_matches('\\').eq_ignore_ascii_case(w))
        }) {
            selected.push(folder.name);
        }
    }
    selected.sort_by_key(|f| (!f.eq_ignore_ascii_case("INBOX"), f.clone()));
    if selected.is_empty() && wanted.is_some() {
        return Err(ApiFailure(
            StatusCode::BAD_REQUEST,
            "folder_mapping_required",
        ));
    }
    Ok(selected)
}
pub(super) async fn search(
    State(app): State<App>,
    Extension(user): Extension<Identity>,
    Query(r): Query<SearchRequest>,
) -> Result<Json<SearchPage>> {
    let owned = app.store.accounts(&user.user).await?;
    let mut accounts: Vec<String> = match &r.accounts {
        Some(ids) if !ids.is_empty() => ids.split(',').map(str::to_owned).collect(),
        _ => owned.into_iter().map(|a| a.id).collect(),
    };
    accounts.sort();
    accounts.dedup();
    let mut scopes = vec![];
    // Resolve every scope before any provider I/O, including continuation requests.
    for id in &accounts {
        scopes.push(app.store.authorize(&user.user, id).await?);
    }
    let (query, wanted, has_attachment) = parse(&r.q)?;
    if r.suggest {
        if r.continuation.is_some() {
            return Err(invalid());
        }
        let page = indexed_or_provider(
            &app,
            &user,
            &r,
            &scopes,
            &query,
            has_attachment,
            wanted.as_deref(),
            &mut Cursor::default(),
        )
        .await;
        return Ok(Json(page.unwrap_or(SearchPage {
            rows: vec![],
            continuation: None,
            coverage: vec![],
            classification: ClassificationCoverage {
                classified: 0,
                total: 0,
            },
            pending: true,
            counts_exact: false,
            cache_preview: Some(true),
            index_page: None,
            search: None,
            supplement: None,
        })));
    }
    let mut cursor: Cursor = match &r.continuation {
        Some(token) if token.len() <= 64 => {
            serde_json::from_str(&app.store.load_search_cursor(&user.user, token).await?)
                .map_err(StorageError::from)?
        }
        Some(_) => return Err(invalid()),
        None => Cursor {
            version: SEARCH_CURSOR_VERSION,
            exclude_trash: r.exclude_trash,
            q: r.q.clone(),
            category: r.category.clone(),
            classification: r.classification(),
            accounts: accounts.clone(),
            ..Default::default()
        },
    };
    if cursor.version != SEARCH_CURSOR_VERSION {
        return Err(ApiFailure(StatusCode::CONFLICT, "refetch_required"));
    }
    if cursor.q != r.q
        || cursor.classification != r.classification()
        || cursor.category != r.category
        || cursor.accounts != accounts
        || cursor.exclude_trash != r.exclude_trash
    {
        return Err(invalid());
    }
    if r.q.trim().eq_ignore_ascii_case("folder:INBOX")
        && !r.exclude_trash
        && (r.continuation.is_none() || cursor.local_inbox)
    {
        cursor.local_inbox = true;
        return inbox(app, user, r, scopes, cursor).await;
    }
    if r.q.trim().eq_ignore_ascii_case("folder:Sent")
        && !r.exclude_trash
        && r.category.is_none()
        && r.classification() == ClassificationFilter::default()
        && (r.continuation.is_none() || cursor.local_sent)
    {
        cursor.local_sent = true;
        return inbox(app, user, r, scopes, cursor).await;
    }
    if r.q.trim().eq_ignore_ascii_case("is:starred")
        && !r.exclude_trash
        && (r.continuation.is_none() || cursor.local_starred)
    {
        cursor.local_starred = true;
        return inbox(app, user, r, scopes, cursor).await;
    }
    if r.q.trim().eq_ignore_ascii_case("is:snoozed")
        && !r.exclude_trash
        && (r.continuation.is_none() || cursor.local_snoozed)
    {
        cursor.local_snoozed = true;
        return inbox(app, user, r, scopes, cursor).await;
    }
    if (r.cache_first || query.text.is_some()) && !r.refresh && r.continuation.is_none() {
        if let Some(page) = indexed_or_provider(
            &app,
            &user,
            &r,
            &scopes,
            &query,
            has_attachment,
            wanted.as_deref(),
            &mut cursor,
        )
        .await
        {
            return Ok(Json(page));
        }
        if let Some(page) = preview::cached(
            &app,
            &user,
            &r,
            &scopes,
            &query,
            has_attachment,
            wanted.as_deref(),
            &cursor,
        )
        .await
        .unwrap_or(None)
        {
            return Ok(Json(page));
        }
    }
    let mut rows = vec![];
    let mut coverage = vec![];
    let mut pending = false;
    if cursor.local_index && !cursor.provider_only {
        if let Some(page) = indexed_or_provider(
            &app,
            &user,
            &r,
            &scopes,
            &query,
            has_attachment,
            wanted.as_deref(),
            &mut cursor,
        )
        .await
        {
            return Ok(Json(page));
        }
        cursor.local_index = false;
    }
    for a in scopes {
        if cursor.provider_only && !cursor.provider_accounts.contains(a.id()) {
            continue;
        }
        let result = async {
            if !cursor.folders.contains_key(a.id()) {
                let mut discovered = app.mail.folders(&app.store, &a).await?;
                if r.exclude_trash || wanted.is_none() || query.text.is_some() {
                    let settings = app.mail.settings(&app.store, &a).await?;
                    let excluded: Vec<String> = discovered
                        .iter()
                        .filter(|folder| {
                            folder.attributes.iter().any(|attr| {
                                matches!(
                                    attr.trim_start_matches('\\').to_ascii_lowercase().as_str(),
                                    "trash" | "junk" | "spam"
                                )
                            }) || settings.trash_folder.as_ref() == Some(&folder.name)
                        })
                        .map(|folder| folder.name.clone())
                        .collect();
                    let mut excluded = excluded;
                    excluded.extend(app.store.search_excluded_folders(&a).await?);
                    excluded.sort();
                    excluded.dedup();
                    discovered.retain(|folder| {
                        !excluded
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(&folder.name))
                    });
                    cursor.excluded_folders.insert(a.id().into(), excluded);
                }
                let folders = select_folders(discovered, wanted.as_deref())?;
                cursor.folders.insert(a.id().into(), folders);
            }
            let folders = cursor.folders.get(a.id()).cloned().unwrap_or_default();
            let position = *cursor.positions.get(a.id()).unwrap_or(&0);
            if let Some(folder) = folders.get(position) {
                let page = app
                    .mail
                    .page(
                        &app.store,
                        &a,
                        folder,
                        query.clone(),
                        cursor.pages.get(a.id()).map(String::as_str),
                    )
                    .await?;
                let complete = page
                    .coverage
                    .iter()
                    .all(|c| c.state == CoverageState::Complete);
                *cursor.full_coverage.entry(a.id().into()).or_insert(true) &= complete;
                app.store
                    .request_classification(
                        &a,
                        &page.rows.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                    )
                    .await?;
                for row in page.rows {
                    if (r.exclude_trash || wanted.is_none() || query.text.is_some())
                        && row.folder_ids.iter().any(|folder| {
                            folder
                                .trim_start_matches('\\')
                                .eq_ignore_ascii_case("Trash")
                                || cursor.excluded_folders.get(a.id()).is_some_and(|excluded| {
                                    excluded
                                        .iter()
                                        .any(|name| name.eq_ignore_ascii_case(folder))
                                })
                        })
                    {
                        continue;
                    }
                    if has_attachment {
                        let content = app.mail.content_background(&app.store, &a, &row.id).await?;
                        if content.attachments.is_empty() {
                            continue;
                        }
                    }
                    let snoozed = app.store.is_snoozed(&a, &row.thread_id).await?;
                    if (wants_snoozed(&r.q) && !snoozed)
                        || (wanted
                            .as_deref()
                            .is_some_and(|f| f.eq_ignore_ascii_case("INBOX"))
                            && snoozed)
                    {
                        continue;
                    }
                    let key = format!("{}:{}", a.id(), row.id);
                    if cursor.seen.insert(key) {
                        // Unknown classification remains visible until the classifier can decide.
                        if r.classification().matches(
                            &row,
                            &chrono::Utc::now()
                                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                        ) && r
                            .category
                            .as_ref()
                            .is_none_or(|c| row.category.as_ref().is_none_or(|v| v == c))
                        {
                            rows.push(row);
                        }
                    }
                }
                match page.continuation {
                    Some(token) => {
                        cursor.pages.insert(a.id().into(), token);
                    }
                    None => {
                        cursor.pages.remove(a.id());
                        cursor.positions.insert(a.id().into(), position + 1);
                    }
                }
            }
            let more = *cursor.positions.get(a.id()).unwrap_or(&0) < folders.len();
            pending |= more;
            Ok::<_, ApiFailure>(AccountCoverage {
                account_id: a.id().into(),
                folder_ids: folders,
                inbox: None,
                state: if more {
                    CoverageState::Pending
                } else if cursor.full_coverage.get(a.id()).copied().unwrap_or(false) {
                    CoverageState::Complete
                } else {
                    CoverageState::Partial
                },
                error: None,
            })
        }
        .await;
        match result {
            Ok(c) => coverage.push(c),
            Err(e) => coverage.push(AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec![],
                inbox: None,
                state: CoverageState::Error,
                error: Some(e.1.into()),
            }),
        }
    }
    rows.sort_by(|a, b| {
        b.received_at
            .cmp(&a.received_at)
            .then_with(|| a.account_id.cmp(&b.account_id))
            .then_with(|| a.id.cmp(&b.id))
    });
    let provider_matches = rows
        .iter()
        .enumerate()
        .map(|(rank, row)| SearchMatch {
            account_id: row.account_id.clone(),
            message_id: row.id.clone(),
            score: 0.5 / (61. + (cursor.provider_rank + rank) as f64),
            excerpt: None,
            attachment: None,
        })
        .collect();
    cursor.provider_rank += rows.len();
    let classified = rows
        .iter()
        .filter(|m| {
            m.classification
                .as_ref()
                .is_some_and(|c| c.status == CategoryStatus::Ready)
        })
        .count() as u32;
    let total = rows.len() as u32;
    let continuation = if pending {
        Some(
            app.store
                .save_search_cursor(
                    &user.user,
                    &serde_json::to_string(&cursor).map_err(StorageError::from)?,
                )
                .await?,
        )
    } else {
        None
    };
    let search = query.text.as_ref().map(|_| SearchMetadata {
        ranked: false,
        matches: provider_matches,
        degradation: coverage
            .iter()
            .map(|c| SearchDegradation {
                account_id: c.account_id.clone(),
                code: "provider_literal_only".into(),
            })
            .collect(),
    });
    Ok(Json(SearchPage {
        rows,
        continuation,
        coverage,
        classification: ClassificationCoverage { classified, total },
        pending,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
        search,
        supplement: None,
    }))
}

async fn sent_coverage(
    app: &App,
    a: &dakia_storage::AuthorizedAccount,
    refresh: bool,
) -> Result<AccountCoverage> {
    let settings = app.mail.settings(&app.store, a).await?;
    let probe: Option<dakia_mail_provider::Probe> = app
        .store
        .provider_document(a, "probe")
        .await?
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let state: Option<crate::sync::State> = app
        .store
        .provider_document(a, "sync:folders")
        .await?
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let discovered = probe
        .as_ref()
        .map(|p| p.folders.as_slice())
        .or_else(|| state.as_ref().map(|s| s.folders.as_slice()));
    let mapping_missing = settings.sent_folder.as_ref().is_some_and(|name| {
        discovered.is_some_and(|folders| !folders.iter().any(|f| f.selectable && f.name == *name))
    });
    let folder_ids = match (settings.sent_folder, discovered) {
        (Some(folder), _) => vec![folder],
        (None, Some(folders)) => {
            dakia_mail_provider::imap::sent_candidates(folders, None).unwrap_or_default()
        }
        (None, None) => vec![],
    };
    if refresh {
        for folder in &folder_ids {
            app.store
                .put_provider_document(a, &format!("sync:head-at:{folder}"), "0")
                .await?;
        }
    }
    let terminal: BTreeMap<_, _> = app
        .store
        .terminal_scan_coverage(a)
        .await?
        .into_iter()
        .collect();
    let (state, error) = if mapping_missing || folder_ids.is_empty() && discovered.is_some() {
        (CoverageState::Error, Some("folder_mapping_required".into()))
    } else if folder_ids.is_empty() || folder_ids.iter().any(|f| !terminal.contains_key(f)) {
        (CoverageState::Pending, None)
    } else if folder_ids.iter().any(|f| terminal[f]) {
        (CoverageState::Partial, None)
    } else {
        (CoverageState::Complete, None)
    };
    Ok(AccountCoverage {
        account_id: a.id().into(),
        folder_ids,
        state,
        error,
        inbox: None,
    })
}

async fn inbox(
    app: App,
    user: Identity,
    r: SearchRequest,
    scopes: Vec<dakia_storage::AuthorizedAccount>,
    mut cursor: Cursor,
) -> Result<Json<SearchPage>> {
    let mut rows = vec![];
    let mut coverage = vec![];
    let mut more = false;
    for a in scopes {
        let sent = if cursor.local_sent {
            Some(sent_coverage(&app, &a, r.refresh && r.continuation.is_none()).await?)
        } else {
            None
        };
        if !cursor.positions.contains_key(a.id()) {
            let before: Option<(String, String)> = cursor
                .pages
                .get(a.id())
                .map(|raw| serde_json::from_str(raw).map_err(StorageError::from))
                .transpose()?;
            let before = before
                .as_ref()
                .map(|(date, id)| (date.as_str(), id.as_str()));
            let mut page = if let Some(sent) = &sent {
                app.store.folder_rows(&a, &sent.folder_ids, before).await?
            } else if cursor.local_starred {
                app.store
                    .starred_rows_filtered(&a, before, r.category.as_deref(), &r.classification())
                    .await?
            } else if cursor.local_snoozed {
                app.store
                    .snoozed_rows_filtered(&a, before, r.category.as_deref(), &r.classification())
                    .await?
            } else {
                app.store
                    .inbox_rows_filtered(&a, before, r.category.as_deref(), &r.classification())
                    .await?
            };
            if page.len() > 50 {
                page.truncate(50);
                if let Some(last) = page.last() {
                    cursor.pages.insert(
                        a.id().into(),
                        serde_json::to_string(&(&last.received_at, &last.id))
                            .map_err(StorageError::from)?,
                    );
                }
                more = true;
            } else {
                cursor.positions.insert(a.id().into(), 1);
            }
            rows.extend(page);
        }
        if let Some(sent) = sent {
            coverage.push(sent);
        } else if cursor.local_starred {
            coverage.push(crate::sync::starred::coverage(&app, &a).await?);
        } else if cursor.local_snoozed {
            coverage.push(AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec![],
                state: CoverageState::Complete,
                error: None,
                inbox: None,
            });
        } else {
            coverage.push(app.store.inbox_coverage(&a).await?);
            app.mail
                .refresh_inbox(&app.store, &a, r.refresh && r.continuation.is_none())
                .await;
        }
    }
    rows.sort_by(|a, b| {
        b.received_at
            .cmp(&a.received_at)
            .then_with(|| a.account_id.cmp(&b.account_id))
            .then_with(|| b.id.cmp(&a.id))
    });
    let total = rows.len() as u32;
    let classified = rows
        .iter()
        .filter(|m| {
            m.classification
                .as_ref()
                .is_some_and(|c| c.status == CategoryStatus::Ready)
        })
        .count() as u32;
    let pending = coverage.iter().any(|c| c.state == CoverageState::Pending);
    let continuation = if more {
        Some(
            app.store
                .save_search_cursor(
                    &user.user,
                    &serde_json::to_string(&cursor).map_err(StorageError::from)?,
                )
                .await?,
        )
    } else {
        None
    };
    Ok(Json(SearchPage {
        rows,
        continuation,
        coverage,
        classification: ClassificationCoverage { classified, total },
        pending,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
        search: None,
        supplement: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_and_fields_are_not_literal_body_terms() {
        let (q, f, has_attachment) =
            parse("folder:Sent is:starred has:attachment from:owner@example.com hello").unwrap();
        assert_eq!(f.as_deref(), Some("Sent"));
        assert_eq!(q.starred, Some(true));
        assert!(has_attachment);
        assert_eq!(q.text.as_deref(), Some("hello"));
    }
    #[test]
    fn special_use_and_unsubscribed_folders_are_selected() {
        let f = || {
            vec![
                dakia_mail_provider::Folder {
                    name: "[Gmail]/Sent Mail".into(),
                    selectable: true,
                    attributes: vec!["\\Sent".into()],
                },
                dakia_mail_provider::Folder {
                    name: "Spam".into(),
                    selectable: true,
                    attributes: vec![],
                },
            ]
        };
        assert_eq!(
            select_folders(f(), Some("Sent")).unwrap(),
            vec!["[Gmail]/Sent Mail"]
        );
        assert_eq!(select_folders(f(), None).unwrap().len(), 2);
        let folders = vec!["Work", "work"]
            .into_iter()
            .map(|name| dakia_mail_provider::Folder {
                name: name.into(),
                selectable: true,
                attributes: vec![],
            })
            .collect();
        assert_eq!(select_folders(folders, Some("Work")).unwrap(), vec!["Work"]);
    }
}
