use super::*;
use dakia_mail_provider::Search;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
mod indexed;
mod preview;

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
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Cursor {
    #[serde(default)]
    local_index: bool,
    #[serde(default)]
    index_after: BTreeMap<String, (String, String)>,
    #[serde(default)]
    index_done: BTreeSet<String>,
    #[serde(default)]
    exclude_trash: bool,
    #[serde(default)]
    local_inbox: bool,
    #[serde(default)]
    local_starred: bool,
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
fn parse(q: &str) -> Result<(Search, Option<String>)> {
    if q.len() > 4096 || q.contains('\0') {
        return Err(invalid());
    }
    let mut search = Search::default();
    let mut folder = None;
    let mut text = vec![];
    let mut terms = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in q.chars() {
        if escaped {
            token.push(c);
            escaped = false;
        } else if c == '\\' && quoted {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            if !token.is_empty() {
                terms.push(std::mem::take(&mut token));
            }
        } else {
            token.push(c);
        }
    }
    if quoted || escaped {
        return Err(invalid());
    }
    if !token.is_empty() {
        terms.push(token);
    }
    for term in &terms {
        match term.split_once(':') {
            Some(("folder", value)) => folder = Some(value.to_owned()),
            Some(("from", value)) => search.from = Some(value.into()),
            Some(("to", value)) => search.to = Some(value.into()),
            Some(("subject", value)) => search.subject = Some(value.into()),
            Some(("is", "starred")) => search.starred = Some(true),
            Some(("is", "read")) => search.seen = Some(true),
            Some(("is", "unread")) => search.seen = Some(false),
            Some(("is", "snoozed")) => {}
            Some(("since" | "after", value)) => search.since = Some(date(value)?),
            Some(("before", value)) => search.before = Some(date(value)?),
            _ => text.push(term.as_str()),
        }
    }
    if !text.is_empty() {
        search.text = Some(text.join(" "));
    }
    search.imap_query().map_err(|_| invalid())?;
    Ok((search, folder))
}
fn date(value: &str) -> Result<String> {
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(date.format("%d-%b-%Y").to_string())
    } else if chrono::NaiveDate::parse_from_str(value, "%d-%b-%Y").is_ok() {
        Ok(value.into())
    } else {
        Err(invalid())
    }
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
    let (query, wanted) = parse(&r.q)?;
    if r.suggest {
        if r.continuation.is_some() {
            return Err(invalid());
        }
        let page = indexed::page(
            &app,
            &user,
            &r,
            &scopes,
            &query,
            wanted.as_deref(),
            &mut Cursor::default(),
        )
        .await?;
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
        })));
    }
    let mut cursor: Cursor = match &r.continuation {
        Some(token) if token.len() <= 64 => {
            serde_json::from_str(&app.store.load_search_cursor(&user.user, token).await?)
                .map_err(StorageError::from)?
        }
        Some(_) => return Err(invalid()),
        None => Cursor {
            exclude_trash: r.exclude_trash,
            q: r.q.clone(),
            category: r.category.clone(),
            classification: r.classification(),
            accounts: accounts.clone(),
            ..Default::default()
        },
    };
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
    if r.q.trim().eq_ignore_ascii_case("is:starred")
        && !r.exclude_trash
        && (r.continuation.is_none() || cursor.local_starred)
    {
        cursor.local_starred = true;
        return inbox(app, user, r, scopes, cursor).await;
    }
    if r.cache_first && r.continuation.is_none() {
        if let Some(page) = indexed::page(
            &app,
            &user,
            &r,
            &scopes,
            &query,
            wanted.as_deref(),
            &mut cursor,
        )
        .await?
        {
            return Ok(Json(page));
        }
        if let Some(page) =
            preview::cached(&app, &user, &r, &scopes, &query, wanted.as_deref(), &cursor).await?
        {
            return Ok(Json(page));
        }
    }
    let mut rows = vec![];
    let mut coverage = vec![];
    let mut pending = false;
    if cursor.local_index {
        if let Some(page) = indexed::page(
            &app,
            &user,
            &r,
            &scopes,
            &query,
            wanted.as_deref(),
            &mut cursor,
        )
        .await?
        {
            return Ok(Json(page));
        }
        cursor.local_index = false;
    }
    for a in scopes {
        let result = async {
            if !cursor.folders.contains_key(a.id()) {
                let mut discovered = app.mail.folders(&app.store, &a).await?;
                if r.exclude_trash {
                    let settings = app.mail.settings(&app.store, &a).await?;
                    let excluded: Vec<String> = discovered
                        .iter()
                        .filter(|folder| {
                            folder.attributes.iter().any(|attr| {
                                attr.trim_start_matches('\\').eq_ignore_ascii_case("Trash")
                            }) || settings.trash_folder.as_ref() == Some(&folder.name)
                        })
                        .map(|folder| folder.name.clone())
                        .collect();
                    discovered.retain(|folder| !excluded.contains(&folder.name));
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
                    if r.exclude_trash
                        && row.folder_ids.iter().any(|folder| {
                            folder
                                .trim_start_matches('\\')
                                .eq_ignore_ascii_case("Trash")
                                || cursor
                                    .excluded_folders
                                    .get(a.id())
                                    .is_some_and(|excluded| excluded.contains(folder))
                        })
                    {
                        continue;
                    }
                    let snoozed = app.store.is_snoozed(&a, &row.thread_id).await?;
                    if (r.q.split_whitespace().any(|t| t == "is:snoozed") && !snoozed)
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
    Ok(Json(SearchPage {
        rows,
        continuation,
        coverage,
        classification: ClassificationCoverage { classified, total },
        pending,
        counts_exact: false,
        cache_preview: None,
        index_page: None,
    }))
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
        if !cursor.positions.contains_key(a.id()) {
            let before: Option<(String, String)> = cursor
                .pages
                .get(a.id())
                .map(|raw| serde_json::from_str(raw).map_err(StorageError::from))
                .transpose()?;
            let before = before
                .as_ref()
                .map(|(date, id)| (date.as_str(), id.as_str()));
            let mut page = if cursor.local_starred {
                app.store
                    .starred_rows_filtered(&a, before, r.category.as_deref(), &r.classification())
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
        if cursor.local_starred {
            coverage.push(crate::sync::starred::coverage(&app, &a).await?);
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
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_and_fields_are_not_literal_body_terms() {
        let (q, f) = parse("folder:Sent is:starred from:owner@example.com hello").unwrap();
        assert_eq!(f.as_deref(), Some("Sent"));
        assert_eq!(q.starred, Some(true));
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
