//! Read-neutral bounded durable text acquisition. Search does not populate the
//! evictable raw/attachment cache and never issues provider mutations.
use super::*;
use mail_parser::MimeHeaders;

pub(crate) fn extract(raw: &[u8]) -> (String, &'static str) {
    let Some(parsed) = mail_parser::MessageParser::default().parse(raw) else {
        return (String::new(), "parse_error");
    };
    if parsed.parts.iter().any(|p| {
        p.content_type().is_some_and(|c| {
            (c.c_type.eq_ignore_ascii_case("multipart")
                && c.c_subtype
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case("encrypted")))
                || c.c_subtype.as_deref().is_some_and(|s| {
                    s.eq_ignore_ascii_case("pkcs7-mime") || s.eq_ignore_ascii_case("pgp-encrypted")
                })
        })
    }) {
        return (String::new(), "encrypted");
    }
    if parsed.parts.iter().any(|p| p.is_encoding_problem) {
        return (String::new(), "parse_error");
    }
    let mut text = String::new();
    // Include both alternatives: a plain-text version need not contain every
    // word in its HTML counterpart. Never index binary payloads or URLs.
    for part in parsed.text_bodies().chain(parsed.html_bodies()) {
        if let Some(value) = part.text_contents() {
            if part.is_text_html() {
                text.push_str(&mail_parser::decoders::html::html_to_text(value));
            } else {
                text.push_str(value);
            }
            text.push('\n');
            if text.len() > 1024 * 1024 {
                return (String::new(), "oversized");
            }
        }
    }
    for part in parsed.attachments() {
        if let Some(name) = part.attachment_name() {
            text.push_str(name);
            text.push('\n');
        }
    }
    (text, "ready")
}

#[tracing::instrument(target = "dakia_server::diagnostics", skip_all, fields(account_key = crate::diagnostics::correlation(a.id()), purpose = "search_index"))]
pub(crate) async fn tick(app: &App, a: &AuthorizedAccount) -> crate::Result<bool> {
    let lane = app.mail.sync_lane(a).await;
    // Foreground actions and metadata always win admission; do not wait behind
    // them while holding a scarce global metadata slot.
    let Ok(_guard) = lane.try_lock() else {
        return Ok(false);
    };
    app.store.index_search_headers(a).await?;
    let now = chrono::Utc::now().timestamp();
    let due = app
        .store
        .provider_document(a, "search:body_due")
        .await?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if due > now {
        return Ok(false);
    }
    // Persist admission before I/O. Failure or process death leaves a bounded
    // delay. A successful bounded page may refill the executor's finite budget.
    // At most 200 candidates and a 4 MiB soft MIME budget. One prefetched
    // small-body chunk or individually capped message can cross that boundary.
    // Yield after two seconds between reads; the account lane and two global
    // jobs still bound concurrent work.
    app.store
        .put_provider_document(a, "search:body_due", &(now + 5).to_string())
        .await?;
    let turn = app
        .store
        .provider_document(a, "search:turn")
        .await?
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    app.store
        .put_provider_document(a, "search:turn", &turn.wrapping_add(1).to_string())
        .await?;
    if turn % 12 == 0 && app.mail.recover_content_reference(&app.store, a).await? {
        return Ok(false);
    }
    let messages = app.store.next_search_bodies(a, turn % 2 == 0, 200).await?;
    let started = std::time::Instant::now();
    let mut bytes = 0;
    let mut failed = false;
    let batch_due = app
        .store
        .provider_document(a, "search:batch_due")
        .await?
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let mut records = Vec::new();
    for chunk in messages.chunks(50) {
        let mut references = Some(Vec::new());
        for message in chunk {
            let reference = if message.provider_id.starts_with("graph:") {
                app.store.provider_document(a, &format!("reference:{}",message.id)).await?
                    .and_then(|raw| serde_json::from_str::<p::MessageRef>(&raw).ok())
            } else { serde_json::from_str::<p::MessageRef>(&message.provider_id).ok() };
            match reference.filter(|r| r.account_id==a.id()) {
                Some(reference) => { if let Some(refs) = &mut references { refs.push(reference); } }
                None => references = None,
            }
        }
        let mut prefetched = None;
        if let Some(refs) = references.filter(|refs| {
            batch_due <= now
                && (refs.iter().all(|r| r.graph_id.is_some()) || refs
                    .iter()
                    .all(|r| r.folder == refs[0].folder && r.uidvalidity == refs[0].uidvalidity))
        }) {
            let provider = app.mail.provider(&app.store, a).await?;
            match provider.fetch_batch(&refs, 4 * 1024 * 1024).await {
                Ok(bodies) if bodies.len() == chunk.len() => prefetched = Some(bodies.into_iter()),
                Err(error)
                    if matches!(
                        error.kind,
                        p::ErrorKind::Unsupported | p::ErrorKind::StaleReference
                    ) => {}
                _ => {
                    // Keep pending work intact after a failed batch. Subsequent
                    // pages use the established individual path to isolate a
                    // bad message, rather than repeatedly stalling its peers.
                    app.store
                        .put_provider_document(a, "search:batch_due", &(now + 300).to_string())
                        .await?;
                    return Err(crate::ApiFailure(
                        axum::http::StatusCode::BAD_GATEWAY,
                        "provider_unavailable",
                    ));
                }
            }
        }
        for m in chunk {
            let result = if let Some(bodies) = &mut prefetched {
                Ok(bodies.next().expect("validated body batch length"))
            } else {
                app.mail
                    .raw_with_limit(&app.store, a, &m.id, 4 * 1024 * 1024)
                    .await
            };
            let (text, state) = match result {
                Ok(raw) => {
                    bytes += raw.len();
                    tokio::task::spawn_blocking(move || extract(&raw))
                        .await
                        .map_err(|_| {
                            crate::ApiFailure(
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                "search_parse_failed",
                            )
                        })?
                }
                Err(error)
                    if matches!(
                        error.1,
                        "content_reference_unavailable" | "content_reference_changed"
                    ) =>
                {
                    // The guarded reference transaction owns this state. Never
                    // overwrite a fresh observation or convert a gap into retry.
                    failed = true;
                    break;
                }
                Err(error) => {
                    let state = if error.0 == axum::http::StatusCode::PAYLOAD_TOO_LARGE
                        || error.1 == "message_too_large"
                    {
                        "oversized"
                    } else {
                        failed = true;
                        "fetch_error"
                    };
                    (String::new(), state)
                }
            };
            tracing::info!(target: "dakia_server::diagnostics", message_key = crate::diagnostics::correlation(&m.id), source_key = crate::diagnostics::correlation(&m.provider_id), operation = "search_index", outcome = state, indexed_bytes = text.len() as u64, "mail_work");
            records.push((m, text, state));
            if failed
                || bytes >= 4 * 1024 * 1024
                || started.elapsed() >= std::time::Duration::from_secs(2)
            {
                break;
            }
        }
        if failed
            || bytes >= 4 * 1024 * 1024
            || started.elapsed() >= std::time::Duration::from_secs(2)
        {
            break;
        }
    }
    // One index transaction per admitted page, rather than one per wire
    // chunk. Repeated FTS segment work and durability barriers are amortized
    // without changing the 200-row, byte, identity or quota bounds.
    let progressed = !records.is_empty();
    if progressed {
        let commits: Vec<_> = records
            .iter()
            .map(|(m, text, state)| (*m, text.as_str(), *state))
            .collect();
        app.store.put_search_bodies(a, &commits).await?;
    }
    if !failed {
        app.store
            .put_provider_document(a, "search:body_due", "0")
            .await?;
    }
    Ok(progressed && !failed)
}
