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
pub(crate) async fn tick(app: &App, a: &AuthorizedAccount) -> crate::Result<()> {
    let lane = app.mail.sync_lane(a).await;
    // Foreground actions and metadata always win admission; do not wait behind
    // them while holding a scarce global metadata slot.
    let Ok(_guard) = lane.try_lock() else {
        return Ok(());
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
        return Ok(());
    }
    // Persist throttle before network I/O. Process death leaves a bounded delay,
    // not a permanently leased job. One read/account/5 seconds, two globally.
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
        return Ok(());
    }
    let Some(m) = app.store.next_search_body(a, turn % 2 == 0).await? else {
        return Ok(());
    };
    let result = app
        .mail
        .raw_with_limit(&app.store, a, &m.id, 4 * 1024 * 1024)
        .await;
    let (text, state) = match result {
        Ok(raw) => tokio::task::spawn_blocking(move || extract(&raw))
            .await
            .map_err(|_| {
                crate::ApiFailure(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "search_parse_failed",
                )
            })?,
        Err(error)
            if matches!(
                error.1,
                "content_reference_unavailable" | "content_reference_changed"
            ) =>
        {
            // The guarded reference transaction owns this state. Do not turn
            // a stale read back into an hourly retry or overwrite a fresh observation.
            tracing::info!(target:"dakia_server::diagnostics",message_key=crate::diagnostics::correlation(&m.id),source_key=crate::diagnostics::correlation(&m.provider_id),operation="search_index",outcome=if error.1=="content_reference_unavailable"{"reference_unavailable"}else{"observation_changed"},"mail_work");
            return Ok(());
        }
        Err(error) => (
            String::new(),
            if error.0 == axum::http::StatusCode::PAYLOAD_TOO_LARGE
                || error.1 == "message_too_large"
            {
                "oversized"
            } else {
                "fetch_error"
            },
        ),
    };
    app.store.put_search_body(a, &m, &text, state).await?;
    tracing::info!(target: "dakia_server::diagnostics", message_key = crate::diagnostics::correlation(&m.id), source_key = crate::diagnostics::correlation(&m.provider_id), operation = "search_index", outcome = state, indexed_bytes = text.len() as u64, "mail_work");
    Ok(())
}
