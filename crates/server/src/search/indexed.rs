use super::*;
use futures_util::StreamExt;
#[path = "indexed_coverage.rs"]
mod coverage_work;

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
    if r.exclude_trash {
        return Ok(None);
    }
    if let Some(page) = hybrid::frozen(app, user, r, scopes, query, wanted, cursor).await? {
        return Ok(Some(page));
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    let plan = dakia_domain::mailbox_query::MailboxQuery::parse(&r.q).map_err(|_| invalid())?;
    let units = plan.text_units();
    let fusion_initial = !r.suggest
        && query.text.is_some()
        && cursor.fused.is_none()
        && app.mail.search_queries.state() == crate::search_query_model::QueryModelState::Ready;
    let vector = if fusion_initial {
        app.mail
            .search_queries
            .embed(&plan.semantic_text())
            .await
            .ok()
    } else {
        None
    };
    let head = if fusion_initial {
        hybrid::LEXICAL_HEAD
    } else {
        50
    };
    let mut semantic = vec![];
    let mut degradation = cursor
        .fused
        .as_ref()
        .map(|f| f.degradation.clone())
        .unwrap_or_default();
    let mut rows = vec![];
    let mut coverage = vec![];
    let mut all_complete = !scopes.is_empty();
    let context = Context {
        app,
        r,
        query,
        wanted,
        units: &units,
        plan: &plan,
        vector: vector.as_ref(),
        fusion_initial,
        head,
        cursor,
        deadline,
    };
    let mut accounts = futures_util::stream::iter(scopes.to_vec())
        .map(|a| async move {
            let outcome = collect_account(&context, &a).await;
            (a.id().to_owned(), outcome)
        })
        .buffer_unordered(dakia_storage::SEARCH_ACCOUNT_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    accounts.sort_by(|a, b| a.0.cmp(&b.0));
    for (id, result) in accounts {
        if let Some(spelling) = result.spelling {
            cursor.spelling.insert(id.clone(), spelling);
        }
        rows.extend(result.rows);
        semantic.extend(result.semantic);
        coverage.extend(result.coverage);
        degradation.extend(result.degradation);
        all_complete &= result.complete;
        if result.done {
            cursor.index_done.insert(id);
        }
    }
    if rows.is_empty() && semantic.is_empty() && !all_complete && query.text.is_none() {
        return Ok(None);
    }
    rows.sort_by(|(a, a_score), (b, b_score)| {
        a_score.total_cmp(b_score).then_with(|| {
            b.received_at
                .cmp(&a.received_at)
                .then_with(|| a.account_id.cmp(&b.account_id))
                .then_with(|| b.id.cmp(&a.id))
        })
    });
    let more = rows.len() > head;
    rows.truncate(if r.suggest { 5 } else { head });
    for (row, score) in &mut rows {
        cursor.index_rank_after.insert(
            row.account_id.clone(),
            (*score, row.received_at.clone(), row.id.clone()),
        );
        cursor.index_after.insert(
            row.account_id.clone(),
            (row.received_at.clone(), row.id.clone()),
        );
    }
    // If every account page fits, there is no unconsumed local row. We fetched
    // 51/account before globally truncating, so no account advances past a row
    // that was not actually returned to the caller.
    if !more {
        for a in scopes {
            cursor.index_done.insert(a.id().into());
        }
    }
    if fusion_initial {
        cursor.fused_lexical_more = more;
        cursor.fused = Some(hybrid::FrozenPage {
            remaining: hybrid::fuse(&rows, semantic),
            coverage: coverage.clone(),
            complete: all_complete,
            degradation: degradation.clone(),
            lexical_rank: rows.len(),
        });
        if cursor
            .fused
            .as_ref()
            .is_some_and(|f| !f.remaining.is_empty())
        {
            return hybrid::frozen(app, user, r, scopes, query, wanted, cursor).await;
        }
    }
    let mut matches = vec![];
    let mut visible = vec![];
    if let Some(frozen) = &cursor.fused {
        all_complete &= frozen.complete;
    }
    for (mut row, score) in rows {
        let rank = if let Some(frozen) = cursor.fused.as_mut() {
            frozen.lexical_rank += 1;
            1. / (60. + frozen.lexical_rank as f64)
        } else {
            -score
        };
        if !cursor.seen.insert(format!("{}:{}", row.account_id, row.id)) {
            continue;
        }
        if query.text.is_some() {
            matches.push(SearchMatch {
                account_id: row.account_id.clone(),
                message_id: row.id.clone(),
                score: rank,
                excerpt: None,
                attachment: None,
            });
        }
        if let Some(a) = scopes.iter().find(|a| a.id() == row.account_id) {
            row = app.store.visible_message(a, row).await?;
        }
        visible.push(row);
    }
    cursor.fused_lexical_more = more;
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
    let rows = visible;
    let total = rows.len() as u32;
    let preview = (!all_complete && !rows.is_empty()).then_some(true);
    let classified = rows
        .iter()
        .filter(|m| {
            m.classification
                .as_ref()
                .is_some_and(|c| c.status == CategoryStatus::Ready)
        })
        .count() as u32;
    Ok(Some(SearchPage {
        rows,
        continuation,
        coverage,
        classification: ClassificationCoverage { classified, total },
        pending: !all_complete,
        counts_exact: false,
        cache_preview: preview,
        index_page: more.then_some(true),
        supplement: None,
        search: query.text.as_ref().map(|_| SearchMetadata {
            ranked: true,
            matches,
            degradation,
        }),
    }))
}

#[derive(Clone, Copy)]
struct Context<'a> {
    app: &'a App,
    r: &'a SearchRequest,
    query: &'a Search,
    wanted: Option<&'a str>,
    units: &'a [String],
    plan: &'a dakia_domain::mailbox_query::MailboxQuery,
    vector: Option<&'a dakia_inference::embedding::Embedding>,
    fusion_initial: bool,
    head: usize,
    cursor: &'a Cursor,
    deadline: tokio::time::Instant,
}
struct AccountPage {
    spelling: Option<Vec<Vec<String>>>,
    rows: Vec<(Message, f64)>,
    semantic: Vec<dakia_storage::SemanticMatch>,
    coverage: Vec<AccountCoverage>,
    degradation: Vec<SearchDegradation>,
    complete: bool,
    done: bool,
}
async fn collect_account(
    context: &Context<'_>,
    a: &dakia_storage::AuthorizedAccount,
) -> AccountPage {
    let Context {
        app,
        r,
        query,
        wanted,
        units,
        plan,
        vector,
        fusion_initial,
        head,
        cursor,
        deadline,
    } = *context;
    let mut rows = vec![];
    let mut semantic = vec![];
    let mut coverage = vec![];
    let mut degradation = vec![];
    let mut all_complete = true;
    let mut done = false;
    let mut spelling = None;
    let mut stage = "folder_state";
    let mut lexical_finished = false;
    let mut semantic_finished = !fusion_initial;

    let coverage_start = coverage.len();
    let mut coverage_started = false;
    let mut coverage_outcome = None;
    let mut known_folders = vec![];
    let outcome = tokio::time::timeout_at(deadline, async {
        let state = app
            .store
            .provider_document(a, "sync:folders")
            .await?
            .and_then(|raw| serde_json::from_str::<crate::sync::State>(&raw).ok());
        stage = "folder_exclusions";
        let excluded = app.store.search_excluded_folders(a).await?;
        let known = state.as_ref().map(|s| {
            s.folders
                .iter()
                .filter(|f| {
                    f.selectable && !excluded.iter().any(|e| e.eq_ignore_ascii_case(&f.name))
                })
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
                        p.folders
                            .into_iter()
                            .filter(|f| {
                                f.selectable
                                    && !excluded.iter().any(|e| e.eq_ignore_ascii_case(&f.name))
                            })
                            .collect(),
                        wanted,
                    )
                    .ok()
                }),
        };
        if wanted.is_some() && folders.is_none() {
            return Err(ApiFailure(
                axum::http::StatusCode::BAD_REQUEST,
                "folder_mapping_required",
            ));
        }
        known_folders = folders.clone().unwrap_or_default();
        coverage_started = true;
        let coverage_read = async {
            coverage_outcome = Some(
                coverage_work::read(
                    app,
                    a,
                    query,
                    state.as_ref().is_some_and(|s| !s.folders.is_empty()),
                    known_folders.clone(),
                )
                .await,
            );
        };
        // Coverage and spelling can fail independently of either retriever.
        // Publish each completed retrieval before waiting for bookkeeping.
        let retrieve = async {
            if cursor.index_done.contains(a.id()) {
                lexical_finished = true;
                semantic_finished = true;
                return Ok(());
            }
            let before = cursor
                .index_after
                .get(a.id())
                .map(|(date, id)| (date.as_str(), id.as_str()));
            stage = "spelling";
            let alternatives = if let Some(saved) = cursor.spelling.get(a.id()) {
                saved.clone()
            } else {
                let allowed: Vec<_> = plan.text.iter().map(|unit| !unit.phrase).collect();
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    app.store.search_spelling(a, units, &allowed),
                )
                .await
                .unwrap_or(Err(StorageError::Invalid))
                {
                    Ok(expanded) => {
                        spelling = Some(expanded.clone());
                        expanded
                    }
                    Err(_) => {
                        all_complete = false;
                        degradation.push(SearchDegradation {
                            account_id: a.id().into(),
                            code: "typo_search_unavailable".into(),
                        });
                        let exact: Vec<_> = units.iter().map(|unit| vec![unit.clone()]).collect();
                        spelling = Some(exact.clone());
                        exact
                    }
                }
            };
            let (since, until) = plan.date_bounds().map_err(|_| invalid())?;
            let filter = &dakia_storage::CachedSearchFilter {
                classification: r.classification(),
                since: since.as_deref(),
                before: until.as_deref(),
                from: query.from.as_deref(),
                to: query.to.as_deref(),
                subject: query.subject.as_deref(),
                text: query.text.as_deref(),
                text_units: Some(units),
                text_alternatives: Some(&alternatives),
                seen: query.seen,
                starred: query.starred,
                folders: if wanted.is_some() {
                    folders.as_deref()
                } else {
                    None
                },
                category: r.category.as_deref(),
                snoozed_only: wants_snoozed(&r.q),
                hide_snoozed: wanted.is_some_and(|f| f.eq_ignore_ascii_case("INBOX")),
            };
            let mut lexical_failed = false;
            let mut semantic_failed = false;
            stage = "retrieval";
            // Each retriever commits its completed positives into the account result
            // before waiting for its sibling. The outer deadline can then cancel
            // stalled work without discarding healthy component results.
            let lexical = async {
                let page = if query.text.is_some() {
                    app.store
                        .full_text_ranked_candidates(
                            a,
                            filter,
                            cursor
                                .index_rank_after
                                .get(a.id())
                                .map(|(s, d, i)| (*s, d.as_str(), i.as_str())),
                            head + 1,
                        )
                        .await
                } else {
                    app.store
                        .full_text_rows(a, filter, before)
                        .await
                        .map(|rows| rows.into_iter().map(|m| (m, 0.0)).collect())
                };
                match page {
                    Ok(page) => {
                        done = page.is_empty();
                        rows.extend(page);
                    }
                    Err(_) => lexical_failed = true,
                }
                lexical_finished = true;
            };
            let vectors = async {
                if !fusion_initial {
                    return;
                }
                let found = if let Some(vector) = vector {
                    app.store
                        .semantic_search_rows_with_phrases(
                            a,
                            filter,
                            &dakia_inference::embedding::search_model_revision(),
                            vector,
                            &plan.phrases(),
                        )
                        .await
                } else {
                    Err(StorageError::Invalid)
                };
                match found {
                    Ok(found) => semantic.extend(found),
                    Err(_) => semantic_failed = true,
                }
                semantic_finished = true;
            };
            tokio::join!(lexical, vectors);
            if lexical_failed {
                all_complete = false;
                degradation.push(SearchDegradation {
                    account_id: a.id().into(),
                    code: "search_index_unavailable".into(),
                });
            }
            if semantic_failed {
                all_complete = false;
                degradation.push(SearchDegradation {
                    account_id: a.id().into(),
                    code: "semantic_search_unavailable".into(),
                });
            }
            Ok::<_, ApiFailure>(())
        };
        let (_, retrieved) = tokio::join!(coverage_read, retrieve);
        retrieved
    })
    .await
    .unwrap_or(Err(ApiFailure(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "search_index_unavailable",
    )));
    if let Some((status, issues)) = coverage_outcome {
        all_complete &= status.state == CoverageState::Complete;
        coverage.push(status);
        degradation.extend(issues);
    } else if coverage_started {
        all_complete = false;
        coverage.push(AccountCoverage {
            account_id: a.id().into(),
            folder_ids: known_folders,
            inbox: None,
            state: CoverageState::Error,
            error: Some("search_coverage_unavailable".into()),
        });
        degradation.push(SearchDegradation {
            account_id: a.id().into(),
            code: "search_coverage_unavailable".into(),
        });
    }
    if let Some(status) = coverage.last_mut() {
        for code in [
            "search_index_unavailable",
            "semantic_search_unavailable",
            "typo_search_unavailable",
        ] {
            if degradation.iter().any(|d| d.code == code) {
                if code == "search_index_unavailable" || status.state != CoverageState::Error {
                    status.state = if code == "search_index_unavailable" {
                        CoverageState::Error
                    } else {
                        CoverageState::Partial
                    };
                    status.error = Some(code.into());
                }
                break;
            }
        }
    }
    if let Err(error) = outcome {
        tracing::warn!(target: "dakia_server::diagnostics", operation = "search_index", stage, lexical_finished, semantic_finished, reason = error.1, "account search did not finish");
        all_complete = false;
        if coverage.len() > coverage_start {
            if let Some(status) = coverage.last_mut() {
                status.state = CoverageState::Error;
                status.error = Some(error.1.into());
            }
        } else {
            coverage.push(AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec![],
                state: CoverageState::Error,
                error: Some(error.1.into()),
                inbox: None,
            });
        }
        degradation.push(SearchDegradation {
            account_id: a.id().into(),
            code: "account_search_unavailable".into(),
        });
    }

    AccountPage {
        spelling,
        rows,
        semantic,
        coverage,
        degradation,
        complete: all_complete,
        done,
    }
}
