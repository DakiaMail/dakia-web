use super::*;

// A local page is either covered or explicitly provisional. The same opaque
// cursor exhausts local keysets first, then traverses the existing provider
// fallback without treating preview IDs as authoritative provider observations.
pub(super) async fn page(
    app: &App,
    user: &Identity,
    r: &SearchRequest,
    scopes: &[dakia_storage::AuthorizedAccount],
    query: &Search,
    wanted: Option<&str>,
    cursor: &mut Cursor,
) -> Result<Option<SearchPage>> {
    if r.exclude_trash || query.since.is_some() || query.before.is_some() {
        return Ok(None);
    }
    let mut rows = vec![];
    let mut coverage = vec![];
    let mut all_complete = !scopes.is_empty();
    let mut available = false;
    for a in scopes {
        let state = app
            .store
            .provider_document(a, "sync:folders")
            .await?
            .and_then(|raw| serde_json::from_str::<crate::sync::State>(&raw).ok());
        let known = state.as_ref().map(|s| {
            s.folders
                .iter()
                .filter(|f| f.selectable)
                .cloned()
                .collect::<Vec<_>>()
        });
        let folders = match known.clone() {
            Some(folders) => select_folders(folders, wanted).ok(),
            None => app
                .store
                .provider_document(a, "probe")
                .await?
                .and_then(|raw| serde_json::from_str::<dakia_mail_provider::Probe>(&raw).ok())
                .and_then(|p| {
                    select_folders(
                        p.folders.into_iter().filter(|f| f.selectable).collect(),
                        wanted,
                    )
                    .ok()
                }),
        };
        if wanted.is_some() && folders.is_none() {
            return Ok(None);
        }
        let text = app.store.text_coverage(a).await?;
        available |= text.headers > 0;
        let fresh = state
            .as_ref()
            .is_some_and(|s| chrono::Utc::now().timestamp() - s.refreshed < 300);
        let complete = fresh
            && text.dirty == 0
            && text.pending == 0
            && text.gaps == 0
            && text.bodies == text.headers
            && app
                .store
                .search_scans_fresh(a, folders.as_deref().unwrap_or_default())
                .await?;
        all_complete &= complete;
        coverage.push(AccountCoverage {
            account_id: a.id().into(),
            folder_ids: folders.clone().unwrap_or_default(),
            inbox: None,
            state: if complete {
                CoverageState::Complete
            } else {
                CoverageState::Partial
            },
            error: None,
        });
        if cursor.index_done.contains(a.id()) {
            continue;
        }
        let before = cursor
            .index_after
            .get(a.id())
            .map(|(date, id)| (date.as_str(), id.as_str()));
        let page = app
            .store
            .full_text_rows(
                a,
                &dakia_storage::CachedSearchFilter {
                    from: query.from.as_deref(),
                    to: query.to.as_deref(),
                    subject: query.subject.as_deref(),
                    text: query.text.as_deref(),
                    seen: query.seen,
                    starred: query.starred,
                    folders: if wanted.is_some() {
                        folders.as_deref()
                    } else {
                        None
                    },
                    category: r.category.as_deref(),
                    snoozed_only: r.q.split_whitespace().any(|t| t == "is:snoozed"),
                    hide_snoozed: wanted.is_some_and(|f| f.eq_ignore_ascii_case("INBOX")),
                },
                before,
            )
            .await?;
        if page.is_empty() {
            cursor.index_done.insert(a.id().into());
        }
        rows.extend(page);
    }
    if !available || (rows.is_empty() && !all_complete) {
        return Ok(None);
    }
    rows.sort_by(|a, b| {
        b.received_at
            .cmp(&a.received_at)
            .then_with(|| a.account_id.cmp(&b.account_id))
            .then_with(|| b.id.cmp(&a.id))
    });
    let more = rows.len() > 50;
    rows.truncate(if r.suggest { 5 } else { 50 });
    for row in &mut rows {
        cursor.index_after.insert(
            row.account_id.clone(),
            (row.received_at.clone(), row.id.clone()),
        );
        if let Some(a) = scopes.iter().find(|a| a.id() == row.account_id) {
            *row = app.store.visible_message(a, row.clone()).await?;
        }
    }
    // If every account page fits, there is no unconsumed local row. We fetched
    // 51/account before globally truncating, so no account advances past a row
    // that was not actually returned to the caller.
    if !more {
        for a in scopes {
            cursor.index_done.insert(a.id().into());
        }
    }
    cursor.local_index = more;
    let continuation = if !r.suggest && (more || !all_complete) {
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
        pending: !all_complete,
        counts_exact: false,
        cache_preview: (!all_complete).then_some(true),
        index_page: more.then_some(true),
    }))
}
