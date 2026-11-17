//! Coverage describes retrieval; it must never gate independent positives.
use super::*;

pub(super) async fn read(
    app: &App,
    a: &dakia_storage::AuthorizedAccount,
    query: &Search,
    has_attachment: bool,
    discovered: bool,
    folders: Vec<String>,
) -> (AccountCoverage, Vec<SearchDegradation>) {
    let mut issues = vec![];
    let (text, mut failed) = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        app.store.text_coverage(a),
    )
    .await
    .unwrap_or(Err(StorageError::Invalid))
    {
        Ok(text) => (text, false),
        Err(_) => {
            issues.push(SearchDegradation {
                account_id: a.id().into(),
                code: "search_coverage_unavailable".into(),
            });
            (dakia_storage::TextCoverage::default(), true)
        }
    };
    let dates_known = if query.since.is_some() || query.before.is_some() {
        match app.store.search_dates_known(a).await {
            Ok(known) => known,
            Err(_) => {
                failed = true;
                issues.push(SearchDegradation {
                    account_id: a.id().into(),
                    code: "search_date_coverage_unavailable".into(),
                });
                false
            }
        }
    } else {
        true
    };
    // Routine refreshes do not expire a durable, caught-up scan. Its source
    // epochs and current eligibility are validated by application storage.
    let ready = dates_known
        && !failed
        && discovered
        && text.lexical_discovered
        && text.lexical_pending == 0
        && !text.lexical_failed
        && text.delivery_pending == 0
        && text.dirty == 0
        && text.pending == 0
        && text.gaps == 0
        && text.bodies == text.headers
        && (!has_attachment || text.attachment_pending == 0);
    let scans_complete = if ready {
        match app.store.search_scans_complete(a, &folders).await {
            Ok(complete) => complete,
            Err(_) => {
                failed = true;
                issues.push(SearchDegradation {
                    account_id: a.id().into(),
                    code: "search_coverage_unavailable".into(),
                });
                false
            }
        }
    } else {
        false
    };
    (
        AccountCoverage {
            account_id: a.id().into(),
            folder_ids: folders,
            inbox: None,
            state: if failed {
                CoverageState::Error
            } else if ready && scans_complete {
                CoverageState::Complete
            } else {
                CoverageState::Partial
            },
            error: failed.then(|| "search_coverage_unavailable".into()),
        },
        issues,
    )
}
