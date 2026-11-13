use crate::diagnostics::correlation;
use crate::App;
use dakia_domain::*;
use dakia_mail_provider as p;
use dakia_storage::{AuthorizedAccount, Error};
use tracing::Instrument;

pub(crate) async fn drain(app: &App, a: &AuthorizedAccount) -> dakia_storage::Result<()> {
    let provider = app.mail.provider(&app.store, a).await.map_err(|_| Error::Invalid)?;
    if let Some(graph) = provider.graph() { return graph_drain(app, a, graph).await; }
    let started = std::time::Instant::now();
    // Avoid the scheduler's five-second sleep between queued mutations while
    // retaining the account lane, per-operation leases and verified outcomes.
    // Never monopolize an account: at most 20 writes or two seconds per turn
    // (an already-started provider call retains its existing deadline).
    for _ in 0..20 {
        if !tick_one(app, a).await? || started.elapsed() >= std::time::Duration::from_secs(2) {
            break;
        }
    }
    Ok(())
}
#[cfg(test)]
pub(crate) async fn tick(app: &App, a: &AuthorizedAccount) -> dakia_storage::Result<()> {
    tick_one(app, a).await.map(|_| ())
}
async fn tick_one(app: &App, a: &AuthorizedAccount) -> dakia_storage::Result<bool> {
    let lane = app.mail.reconciliation_lane(a).await;
    let waiting = std::time::Instant::now();
    let _guard = lane.lock().await;
    let lane_wait_ms = waiting.elapsed().as_millis() as u64;
    let Some(mut op) = app.store.claim_mutation(a).await? else {
        return Ok(false);
    };
    // Opaque correlation keys only, never client IDs or message content.
    let span = tracing::info_span!(
        "mail_mutation",
        account_key = correlation(a.id()),
        operation_key = correlation(&op.id),
        message_key = correlation(&op.target_id),
        action = action_name(&op.change)
    );
    async {
        let started = std::time::Instant::now();
        tracing::info!(stage = "claimed", lane_wait_ms, "mail mutation claimed");
        let mut folders = None;
        let mut safe_to_retry = false;
        match apply(app, a, &op, &mut safe_to_retry).await {
            Ok(f) => {
                op.status = OperationStatus::Applied;
                folders = Some(f);
            }
            Err(error) => {
                if safe_to_retry && app.store.retry_mutation(a, &op.id).await? {
                    tracing::info!("mail action deferred before provider write");
                    return Ok(());
                }
                op.status = OperationStatus::Failed;
                tracing::warn!(
                    // Storage Error's Display is a static category, unlike Debug.
                    error = %error,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "mail mutation not verified; no automatic replay"
                );
            }
        }
        let status = op.status.clone();
        app.store.finish_mutation(a, op, folders).await.map_err(|error| {
            tracing::warn!(stage = "persist_outcome", error = %error, "mail mutation outcome not persisted");
            error
        })?;
        tracing::info!(
            ?status,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "mail mutation outcome persisted"
        );
        Ok::<(), Error>(())
    }
    .instrument(span)
    .await?;
    Ok(true)
}
async fn graph_drain(app: &App, a: &AuthorizedAccount, graph: &p::GraphProvider) -> dakia_storage::Result<()> {
    let lane = app.mail.reconciliation_lane(a).await;
    let _guard = lane.lock().await;
    let mut operations = Vec::new();
    let mut changes = Vec::new();
    for _ in 0..20 {
        let Some(mut operation) = app.store.claim_mutation(a).await? else { break };
        let prepared = async {
            let message = app.store.message(a,&operation.target_id).await?;
            let reference: p::MessageRef = serde_json::from_str(&app.store.provider_document(a,&format!("reference:{}",message.id)).await?.ok_or(Error::NotFound)?)?;
            if reference.account_id!=a.id() || reference.graph_id.is_none() { return Err(Error::Invalid); }
            let change = match &operation.change {
                Change::Read { value } => p::Mutation::Seen(*value),
                Change::Star { value } => p::Mutation::Starred(*value),
                Change::Archive => p::Mutation::Archive, Change::Trash => p::Mutation::Trash, Change::Spam => p::Mutation::Spam,
                Change::Move { folder_id } => p::Mutation::Move(folder_id.clone()),
                _ => return Err(Error::Invalid),
            };
            Ok((message,reference,change))
        }.await;
        match prepared {
            Ok((message,reference,change)) => {
                changes.push((reference.clone(),change));
                operations.push((operation,message,reference));
            }
            Err(_) => { operation.status=OperationStatus::Failed; app.store.finish_mutation(a,operation,None).await?; }
        }
    }
    if changes.is_empty() { return Ok(()); }
    let results = match graph.mutate_batch(&changes).await {
        Ok(results) if results.len()==changes.len() => results,
        Ok(_) => vec![Err(p::error(p::ErrorKind::Uncertain,"Graph batch outcome unknown")); changes.len()],
        Err(e) => vec![Err(e); changes.len()],
    };
    for ((mut operation,mut message,reference),result) in operations.into_iter().zip(results) {
        match result {
            Ok(result) => {
                if let Some(destination) = result.destination {
                    message.folder_ids.retain(|f| f!=&reference.folder);
                    if !message.folder_ids.contains(&destination) { message.folder_ids.push(destination); }
                }
                if let Some(reference) = result.destination_reference {
                    let id = reference.graph_id.as_ref().ok_or(Error::Invalid)?;
                    app.store.link_identity(a,&format!("graph:{id}"),&message.id).await?;
                    app.store.put_provider_document(a,&format!("reference:{}",message.id),&serde_json::to_string(&reference)?).await?;
                }
                operation.status=OperationStatus::Applied;
                app.store.finish_mutation(a,operation,Some(message.folder_ids)).await?;
            }
            Err(e) => {
                if e.kind==p::ErrorKind::Retryable && app.store.retry_mutation(a,&operation.id).await? { continue; }
                crate::mail::provider_error(e);
                operation.status=OperationStatus::Failed;
                app.store.finish_mutation(a,operation,None).await?;
            }
        }
    }
    Ok(())
}
pub(crate) fn action_name(change: &Change) -> &'static str {
    match change {
        Change::Read { value: true } => "mark_read",
        Change::Read { value: false } => "mark_unread",
        Change::Star { value: true } => "star",
        Change::Star { value: false } => "unstar",
        Change::Archive => "archive",
        Change::Trash => "trash",
        Change::Spam => "spam",
        Change::Move { .. } => "move",
        _ => "local_change",
    }
}
fn provider_failure(
    stage: &'static str,
    error: p::ProviderError,
    safe_to_retry: &mut bool,
) -> Error {
    // This mapper logs allowlisted reasons/kinds, not raw provider text.
    let _stage = tracing::warn_span!("mutation_provider", stage).entered();
    *safe_to_retry = error.kind == p::ErrorKind::Retryable;
    let provider_outcome = if error.kind == p::ErrorKind::Uncertain {
        "unknown"
    } else {
        "failed"
    };
    tracing::warn!(provider_outcome, "mail mutation provider outcome");
    crate::mail::provider_error(error);
    Error::Invalid
}
async fn apply(
    app: &App,
    a: &AuthorizedAccount,
    op: &Operation,
    safe_to_retry: &mut bool,
) -> dakia_storage::Result<Vec<String>> {
    let mut message = app.store.message(a, &op.target_id).await?;
    // A preceding queued action may have failed, leaving this restore already satisfied.
    if matches!(&op.change, Change::Move { folder_id } if folder_id.eq_ignore_ascii_case("INBOX"))
        && message
            .folder_ids
            .iter()
            .any(|folder| folder.eq_ignore_ascii_case("INBOX"))
    {
        return Ok(message.folder_ids);
    }
    // Archive via the observed INBOX membership, not whichever Gmail label
    // happened to be scanned last. Never substitute a UID from another folder.
    let inbox_reference =
        if matches!(op.change, Change::Archive) && message.provider_id.starts_with("gmail:") {
            app.store
                .provider_document(a, &format!("member-reference:{}:INBOX", message.id))
                .await?
        } else {
            None
        };
    let raw = match inbox_reference {
        Some(raw) => raw,
        None => app
            .store
            .provider_document(a, &format!("reference:{}", message.id))
            .await?
            .ok_or(Error::NotFound)?,
    };
    let mut reference: p::MessageRef = serde_json::from_str(&raw)?;
    if reference.account_id != a.id() {
        return Err(Error::Invalid);
    }
    if message.provider_id.starts_with("gmail:")
        && !matches!(op.change, Change::Archive)
        && !message.folder_ids.contains(&reference.folder)
    {
        for raw in app
            .store
            .message_reference_documents(a, &message.id)
            .await?
        {
            let candidate: p::MessageRef = serde_json::from_str(&raw)?;
            if candidate.account_id == a.id()
                && message.folder_ids.contains(&candidate.folder)
                && candidate.uid > 0
            {
                let epoch = app
                    .store
                    .provider_document(a, &format!("thread:epoch:{}", candidate.folder))
                    .await?;
                if epoch.is_none_or(|value| value == candidate.uidvalidity.to_string()) {
                    reference = candidate;
                    break;
                }
            }
        }
    }
    let mutation = match &op.change {
        Change::Read { value } => p::Mutation::Seen(*value),
        Change::Star { value } => p::Mutation::Starred(*value),
        Change::Archive => p::Mutation::Archive,
        Change::Trash => p::Mutation::Trash,
        Change::Spam => p::Mutation::Spam,
        Change::Move { folder_id } => p::Mutation::Move(folder_id.clone()),
        _ => return Err(Error::Invalid),
    };
    let provider = app.mail.provider(&app.store, a).await.map_err(|_| {
        tracing::warn!(
            stage = "provider_setup",
            "mail mutation provider unavailable"
        );
        Error::Invalid
    })?;
    // Discover roles before the irreversible write, not after a successful
    // verified move where a failed LIST would discard the destination reference.
    let gmail_move_folders = if matches!(op.change, Change::Spam | Change::Trash)
        && message.provider_id.starts_with("gmail:")
    {
        Some(
            provider
                .folders()
                .await
                .map_err(|e| provider_failure("folder_discovery", e, safe_to_retry))?,
        )
    } else {
        None
    };
    let result = match provider.mutate(&reference, mutation.clone()).await {
        Ok(result) => result,
        Err(error) if error.kind == p::ErrorKind::Uncertain => {
            // Exactly one bounded readback, never another write. Stable Gmail
            // identity allows destination proof even when COPYUID was lost.
            let identity = if let Some(id) = message.provider_id.strip_prefix("gmail:") {
                p::Identity::Gmail {
                    account_id: a.id().into(),
                    message_id: id.parse().map_err(|_| Error::Invalid)?,
                }
            } else {
                p::Identity::Imap(reference.clone())
            };
            crate::mail::provider_error(error.clone());
            match provider
                .verify_mutation(&reference, &identity, &mutation)
                .await
            {
                Ok(Some(result)) => {
                    tracing::info!(
                        stage = "uncertain_readback",
                        outcome = "verified",
                        "mail mutation recovered without replay"
                    );
                    result
                }
                readback => {
                    if let Err(e) = readback {
                        crate::mail::provider_error(e);
                    }
                    return Err(provider_failure("uncertain_readback", error, safe_to_retry));
                }
            }
        }
        Err(error) => return Err(provider_failure("mutate_or_verify", error, safe_to_retry)),
    };
    if let Some(destination) = result.destination {
        message.folder_ids.retain(|f| f != &reference.folder);
        if let Some(folders) = gmail_move_folders {
            // Gmail Trash/Junk exclude Inbox and All Mail. Preserve unrelated
            // user labels, but remove the mutually exclusive special-use role.
            let excluded = if matches!(op.change, Change::Trash) {
                "junk"
            } else {
                "trash"
            };
            message.folder_ids.retain(|name| {
                !name.eq_ignore_ascii_case("INBOX")
                    && name != "\\Inbox"
                    && !folders.iter().any(|f| {
                        f.name == *name
                            && f.attributes.iter().any(|role| {
                                let role = role.trim_start_matches('\\').to_ascii_lowercase();
                                role == "all" || role == excluded
                            })
                    })
            });
        }
        if !message.folder_ids.contains(&destination) {
            message.folder_ids.push(destination);
        }
    } else if matches!(op.change, Change::Archive) {
        message
            .folder_ids
            .retain(|f| !f.eq_ignore_ascii_case("INBOX") && f != "\\Inbox");
    }
    if let Some(reference) = result.destination_reference {
        let raw = serde_json::to_string(&reference)?;
        // Generic IMAP identity changes on MOVE but Dakia-owned state does not.
        if let Some(id) = &reference.graph_id {
            app.store.link_identity(a, &format!("graph:{id}"), &message.id).await?;
        } else if !message.provider_id.starts_with("gmail:") {
            app.store.link_identity(a, &raw, &message.id).await?;
        }
        app.store
            .put_provider_document(a, &format!("reference:{}", message.id), &raw)
            .await?;
    }
    Ok(message.folder_ids)
}
