//! Bounded, explicitly provisional local positives. Provider continuation is
//! authoritative and does not inherit preview IDs as already seen.
use super::*;

pub(super) async fn cached(
    app: &App,
    user: &Identity,
    r: &SearchRequest,
    scopes: &[dakia_storage::AuthorizedAccount],
    query: &Search,
    wanted: Option<&str>,
    cursor: &Cursor,
) -> Result<Option<SearchPage>> {
    // INTERNALDATE calendar-day semantics cannot be reconstructed from the UTC
    // projection. Cleanup already has an exact, trash-aware local catalogue.
    if r.exclude_trash || query.since.is_some() || query.before.is_some() {
        return Ok(None);
    }
    let mut rows = vec![];
    let mut coverage = vec![];
    for a in scopes {
        let folders = match app.store.provider_document(a, "probe").await? {
            Some(raw) => serde_json::from_str::<dakia_mail_provider::Probe>(&raw)
                .ok()
                .and_then(|p| {
                    select_folders(
                        p.folders.into_iter().filter(|f| f.selectable).collect(),
                        wanted,
                    )
                    .ok()
                }),
            None => None,
        };
        // Never guess localized special-use roles or case-fold custom folders.
        if wanted.is_some() && folders.is_none() {
            coverage.push(AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec![],
                state: CoverageState::Pending,
                error: None,
                inbox: None,
            });
            continue;
        }
        rows.extend(
            app.store
                .cached_search_rows(
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
                )
                .await?,
        );
        coverage.push(AccountCoverage {
            account_id: a.id().into(),
            folder_ids: folders.unwrap_or_default(),
            state: CoverageState::Pending,
            error: None,
            inbox: None,
        });
    }
    if rows.is_empty() {
        return Ok(None);
    }
    rows.sort_by(|a, b| {
        b.received_at
            .cmp(&a.received_at)
            .then_with(|| a.account_id.cmp(&b.account_id))
            .then_with(|| b.id.cmp(&a.id))
    });
    let continuation = Some(
        app.store
            .save_search_cursor(
                &user.user,
                &serde_json::to_string(cursor).map_err(StorageError::from)?,
            )
            .await?,
    );
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
        pending: true,
        counts_exact: false,
        cache_preview: Some(true),
        index_page: None,
    }))
}
