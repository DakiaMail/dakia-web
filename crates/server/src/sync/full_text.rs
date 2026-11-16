//! Read-neutral bounded durable text acquisition. Search does not populate the
//! evictable raw/attachment cache and never issues provider mutations.
use super::*;
use mail_parser::MimeHeaders;

pub(crate) struct Extracted {
    pub has_attachment: Option<bool>,
    pub text: String,
    pub participants: String,
    pub recipients: String,
    pub attachments: String,
    pub details: String,
    pub state: &'static str,
}

impl Default for Extracted {
    fn default() -> Self {
        Self {
            has_attachment: None,
            text: String::new(),
            participants: String::new(),
            recipients: String::new(),
            attachments: String::new(),
            details: "[]".into(),
            state: "parse_error",
        }
    }
}

#[cfg(test)]
pub(crate) fn extract(raw: &[u8]) -> (String, &'static str, Option<bool>) {
    let parsed = extract_content(raw);
    (
        format!(
            "{}\n{}\n{}",
            parsed.text, parsed.participants, parsed.attachments
        ),
        parsed.state,
        matches!(parsed.state, "ready" | "partial" | "oversized").then(|| parsed.details != "[]"),
    )
}

pub(crate) fn extract_content(raw: &[u8]) -> Extracted {
    let Some(parsed) = mail_parser::MessageParser::default().parse(raw) else {
        return Extracted {
            state: "parse_error",
            ..Default::default()
        };
    };
    let mut encrypted_parts = vec![false; parsed.parts.len()];
    let mut encrypted_roots: Vec<_> = parsed
        .parts
        .iter()
        .enumerate()
        .filter_map(|(index, p)| {
            p.content_type()
                .is_some_and(|c| {
                    (c.c_type.eq_ignore_ascii_case("multipart")
                        && c.c_subtype
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case("encrypted")))
                        || c.c_subtype.as_deref().is_some_and(|s| {
                            s.eq_ignore_ascii_case("pkcs7-mime")
                                || s.eq_ignore_ascii_case("pgp-encrypted")
                        })
                })
                .then_some(index)
        })
        .collect();
    while let Some(index) = encrypted_roots.pop() {
        let Some(part) = parsed.parts.get(index) else {
            continue;
        };
        if std::mem::replace(&mut encrypted_parts[index], true) {
            continue;
        }
        if let mail_parser::PartType::Multipart(children) = &part.body {
            encrypted_roots.extend(children.iter().map(|id| *id as usize));
        }
    }
    let encrypted = encrypted_parts.first().copied().unwrap_or(false);
    let mut text = String::new();
    let mut gaps = encrypted_parts.iter().any(|encrypted| *encrypted);
    let mut oversized = false;
    // Include both alternatives: a plain-text version need not contain every
    // word in its HTML counterpart. Binary attachment payloads do not enter the body index.
    for part in parsed
        .text_body
        .iter()
        .chain(&parsed.html_body)
        .filter(|id| !encrypted_parts[**id as usize])
        .filter_map(|id| parsed.parts.get(*id as usize))
    {
        if part.is_encoding_problem {
            gaps = true;
            continue;
        }
        if let Some(value) = part.text_contents() {
            let decoded = if part.is_text_html() {
                mail_parser::decoders::html::html_to_text(value)
            } else {
                value.to_owned()
            };
            let remaining = (1024 * 1024usize).saturating_sub(text.len());
            let mut end = decoded.len().min(remaining);
            while !decoded.is_char_boundary(end) {
                end -= 1;
            }
            text.push_str(&decoded[..end]);
            if decoded.len() >= remaining {
                oversized = true;
                gaps = true;
                break;
            }
            text.push('\n');
        }
    }
    let addresses = |list: Option<&mail_parser::Address<'_>>| -> String {
        list.into_iter()
            .flat_map(|a| a.iter())
            .map(|a| {
                format!(
                    "{} {}",
                    a.name.as_deref().unwrap_or(""),
                    a.address.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut recipients = [
        addresses(parsed.to()),
        addresses(parsed.cc()),
        addresses(parsed.bcc()),
    ]
    .join("\n");
    let mut participants = [
        addresses(parsed.from()),
        recipients.clone(),
        addresses(parsed.reply_to()),
    ]
    .join("\n");
    if participants.len() + recipients.len() > 1024 * 1024 {
        participants.clear();
        recipients.clear();
        gaps = true;
    }
    let mut attachments = String::new();
    let mut details = Vec::new();
    for (index, id) in parsed.attachments.iter().enumerate() {
        let Some(part) = parsed.parts.get(*id as usize) else {
            gaps = true;
            continue;
        };
        let part_encrypted = encrypted_parts[*id as usize];
        if index >= 512 {
            gaps = true;
            details.push(serde_json::json!({"state":"resource_blocked","filename":null}));
            break;
        }
        let name = part.attachment_name().unwrap_or("attachment");
        if name.len() > 64 * 1024 {
            gaps = true;
            details
                .push(serde_json::json!({"part":index,"filename":null,"state":"resource_blocked"}));
            continue;
        }

        let format = anydoc::Format::from_bytes(part.contents()).or_else(|| {
            name.rsplit_once('.')
                .and_then(|(_, ext)| anydoc::Format::from_extension(ext))
        });
        let result = if part_encrypted {
            Err("encrypted")
        } else if part.is_encoding_problem {
            Err("decode_failed")
        } else if part.contents().len() > 16 * 1024 * 1024 {
            Err("resource_blocked")
        } else if let Some(format) = format {
            crate::mail::attachment_bytes(raw, part)
                .map_err(|_| "decode_failed")
                .and_then(|bytes| {
                    anydoc::to_markdown_bytes(&bytes, format).map_err(|error| match error {
                        anydoc::ConvertError::NeedsOcr { .. } => "needs_ocr",
                        anydoc::ConvertError::ResourceLimit { .. } => "resource_blocked",
                        _ => "extraction_failed",
                    })
                })
        } else if part
            .content_type()
            .is_some_and(|t| t.c_type.eq_ignore_ascii_case("text"))
        {
            part.text_contents()
                .map(str::to_owned)
                .ok_or("extraction_failed")
        } else {
            Err("unsupported")
        };
        let (value, mut status) = match result {
            Ok(value) if attachments.len() + value.len() <= 8 * 1024 * 1024 => (value, "ready"),
            Ok(_) => {
                gaps = true;
                (String::new(), "resource_blocked")
            }
            Err(status) => {
                gaps = true;
                (String::new(), status)
            }
        };
        // Offsets refer to the normalized text retained by search storage.
        // Normalize each newline-delimited part before measuring its span.
        let normalized_name = dakia_storage::normalize_search(name);
        let value = dakia_storage::normalize_search(&value);
        let byte_start = attachments.len();
        if attachments.len() + normalized_name.len() + value.len() + 2 <= 8 * 1024 * 1024 {
            attachments.push_str(&normalized_name);
            attachments.push('\n');
            attachments.push_str(&value);
            attachments.push('\n');
        } else {
            gaps = true;
            status = "resource_blocked";
        }

        details.push(serde_json::json!({"part": index, "filename": name.chars().take(200).collect::<String>(), "state": status, "byte_start":byte_start, "byte_end":attachments.len()}));
    }
    Extracted {
        has_attachment: Some(!parsed.attachments.is_empty()),
        text,
        participants,
        recipients,
        attachments,
        details: serde_json::to_string(&details).expect("attachment metadata serialization"),
        state: if encrypted {
            "encrypted"
        } else if oversized {
            "oversized"
        } else if gaps {
            "partial"
        } else {
            "ready"
        },
    }
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
    if !deliver_index(app, a).await? {
        return Ok(false);
    }
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
    let mut extracted_bytes = 0;
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
                app.store
                    .provider_document(a, &format!("reference:{}", message.id))
                    .await?
                    .and_then(|raw| serde_json::from_str::<p::MessageRef>(&raw).ok())
            } else {
                serde_json::from_str::<p::MessageRef>(&message.provider_id).ok()
            };
            match reference.filter(|r| r.account_id == a.id()) {
                Some(reference) => {
                    if let Some(refs) = &mut references {
                        refs.push(reference);
                    }
                }
                None => references = None,
            }
        }
        let mut prefetched = None;
        if let Some(refs) = references.filter(|refs| {
            batch_due <= now
                && (refs.iter().all(|r| r.graph_id.is_some())
                    || refs.iter().all(|r| {
                        r.folder == refs[0].folder && r.uidvalidity == refs[0].uidvalidity
                    }))
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
                    .raw_with_limit(&app.store, a, &m.id, crate::mail::RECEIVED_MESSAGE_LIMIT)
                    .await
            };
            let extracted = match result {
                Ok(raw) => {
                    bytes += raw.len();
                    tokio::task::spawn_blocking(move || extract_content(&raw))
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
                    Extracted {
                        state,
                        ..Default::default()
                    }
                }
            };
            tracing::info!(target: "dakia_server::diagnostics", message_key = crate::diagnostics::correlation(&m.id), source_key = crate::diagnostics::correlation(&m.provider_id), operation = "search_index", outcome = extracted.state, indexed_bytes = extracted.text.len() as u64, "mail_work");
            extracted_bytes += extracted.text.len() + extracted.attachments.len();
            records.push((m, extracted));
            if failed
                || bytes >= 4 * 1024 * 1024
                || extracted_bytes >= 8 * 1024 * 1024
                || started.elapsed() >= std::time::Duration::from_secs(2)
            {
                break;
            }
        }
        if failed
            || bytes >= 4 * 1024 * 1024
            || extracted_bytes >= 8 * 1024 * 1024
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
            .map(|(message, content)| dakia_storage::SearchContent {
                has_attachment: content.has_attachment,
                message,
                text: &content.text,
                state: content.state,
                participants: &content.participants,
                recipients: &content.recipients,
                attachments: &content.attachments,
                attachment_details: &content.details,
            })
            .collect();
        app.store.put_search_records(a, &commits).await?;
        if !deliver_index(app, a).await? {
            return Ok(false);
        }
    }
    if !failed {
        app.store
            .put_provider_document(a, "search:body_due", "0")
            .await?;
    }
    Ok(progressed && !failed)
}

async fn deliver_index(app: &App, account: &AuthorizedAccount) -> crate::Result<bool> {
    match app.store.flush_search_index(account).await {
        Ok(_) => {
            app.store
                .put_provider_document(account, "search:delivery_error", "")
                .await?;
            return Ok(true);
        }
        Err(_) => {
            tracing::warn!(target: "dakia_server::diagnostics", operation="search_index", outcome="delivery_failed", "search delivery will retry");
            app.store
                .put_provider_document(account, "search:delivery_error", "unavailable")
                .await?;
        }
    }
    Ok(false)
}
