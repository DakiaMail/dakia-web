//! Full membership snapshots may contain lightweight mutable observations.
//! A CONDSTORE delta alone is never returned to record_scan as a snapshot.
use super::*;

pub(super) fn reference_key(r: &p::MessageRef) -> String {
    if let Some(id) = &r.graph_id {
        return format!("sync:graph-ref:{}:{}:{id}", r.folder.len(), r.folder);
    }
    // Length framing keeps user-controlled mailbox names unambiguous.
    format!(
        "sync:ref:{}:{}:{}:{}",
        r.folder.len(),
        r.folder,
        r.uidvalidity,
        r.uid
    )
}
fn provider_id(identity: &p::Identity, a: &AuthorizedAccount) -> Result<String> {
    match identity {
        p::Identity::Gmail {
            account_id,
            message_id,
        } if account_id == a.id() => Ok(format!("gmail:{message_id}")),
        p::Identity::Graph { account_id, message_id } if account_id == a.id() => Ok(format!("graph:{message_id}")),
        p::Identity::Imap(r) if r.account_id == a.id() => serde_json::to_string(r)
            .map_err(Error::from)
            .map_err(Into::into),
        _ => Err(ApiFailure(StatusCode::FORBIDDEN, "account_mismatch")),
    }
}
struct Lookup<'a> {
    store: &'a Store,
    account: &'a AuthorizedAccount,
    // Sent metadata carries recipient-learning evidence (including Cc/Bcc).
    // Fetch complete metadata until that particular folder ref was ingested.
    sent: bool,
    drafts: bool,
}
impl Lookup<'_> {
    async fn referenced(&self, r: &p::MessageRef) -> dakia_storage::Result<Option<Message>> {
        if r.account_id != self.account.id() {
            return Err(Error::Invalid);
        }
        let Some(id) = self
            .store
            .provider_document(self.account, &reference_key(r))
            .await?
        else {
            return Ok(None);
        };
        match self.store.message(self.account, &id).await {
            Ok(m) => Ok(Some(m)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
#[async_trait::async_trait]
impl p::MetadataLookup for Lookup<'_> {
    async fn identity_known(&self, identity: &p::Identity) -> p::Result<bool> {
        if self.drafts || self.sent {
            return Ok(false);
        }
        let id = provider_id(identity, self.account)
            .map_err(|_| p::error(p::ErrorKind::InvalidInput, "metadata scope mismatch"))?;
        self.store
            .known_provider_message(self.account, &id)
            .await
            .map(|m| m.is_some())
            .map_err(|_| p::error(p::ErrorKind::Retryable, "metadata lookup failed"))
    }
    async fn reference_known(&self, reference: &p::MessageRef) -> p::Result<bool> {
        if self.drafts {
            return Ok(false);
        }
        self.referenced(reference)
            .await
            .map(|m| m.is_some())
            .map_err(|_| p::error(p::ErrorKind::Retryable, "metadata lookup failed"))
    }
}
impl MailRuntime {
    /// The bool reports a successful provider check, so the scheduler charges
    /// it one turn instead of checking every completed folder in a burst.
    pub(crate) async fn prepare_scan(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
    ) -> Result<(Option<(i64, Option<String>)>, bool)> {
        let lane = self.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        if let Some(generation) = store.completed_scan_due(a, folder).await? {
            let checkpoint: Option<p::ModseqCheckpoint> = store
                .provider_document(a, &format!("sync:modseq:{folder}"))
                .await?
                .and_then(|raw| serde_json::from_str(&raw).ok());
            if let Some(checkpoint) =
                checkpoint.filter(|c| c.account_id == a.id() && c.folder == folder)
            {
                let provider = self.provider(store, a).await?;
                let _activity = self.begin_sync(a).await;
                if provider
                    .snapshot_unchanged(&checkpoint)
                    .await
                    .map_err(provider_error)?
                {
                    store.confirm_unchanged_scan(a, folder, generation).await?;
                    return Ok((None, true));
                }
            }
        }
        Ok((store.scan_state(a, folder).await?, false))
    }

    #[tracing::instrument(target = "dakia_server::diagnostics", skip_all, fields(account_key = crate::diagnostics::correlation(a.id()), folder_key = crate::diagnostics::correlation(folder)))]
    pub(super) async fn reconcile_page(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        folder: &str,
        continuation: Option<&str>,
        record_generation: Option<i64>,
    ) -> Result<SearchPage> {
        let lane = self.reconciliation_lane(a).await;
        let _guard = lane.lock().await;
        if let Some(generation) = record_generation {
            if store.scan_generation(a, folder).await? != Some(generation) {
                return Err(Error::Conflict.into());
            }
        }
        let epoch_key = format!("thread:epoch:{folder}");
        let starting_epoch = store.provider_document(a, &epoch_key).await?;
        let token: Option<p::PageToken> = continuation
            .map(|s| {
                URL_SAFE_NO_PAD
                    .decode(s)
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .ok_or(ApiFailure(StatusCode::BAD_REQUEST, "invalid_continuation"))
            })
            .transpose()?;
        let key = format!("sync:modseq:{folder}");
        let since: Option<p::ModseqCheckpoint> = store
            .provider_document(a, &key)
            .await?
            .and_then(|s| serde_json::from_str(&s).ok());
        let probe: Option<p::Probe> = store
            .provider_document(a, "probe")
            .await?
            .and_then(|s| serde_json::from_str(&s).ok());
        let role = |name: &str| {
            probe.as_ref().is_some_and(|p| {
                p.folders.iter().any(|f| {
                    f.name == folder
                        && f.attributes
                            .iter()
                            .any(|a| a.trim_start_matches('\\').eq_ignore_ascii_case(name))
                })
            })
        };
        let lookup = Lookup {
            store,
            account: a,
            sent: role("Sent")
                || self.settings(store, a).await?.sent_folder.as_deref() == Some(folder),
            drafts: role("Drafts"),
        };
        let provider = self.provider(store, a).await?;
        let _activity = self.begin_sync(a).await;
        let observed_after = chrono::Utc::now().timestamp_micros();
        let page = provider
            .reconcile(folder, token.as_ref(), 200, &lookup, since.as_ref())
            .await
            .map_err(provider_error)?;
        if page.rows.len() > 200 {
            return Err(ApiFailure(
                StatusCode::BAD_GATEWAY,
                "invalid_provider_metadata",
            ));
        }
        let mut full = Vec::new();
        let mut mutable = Vec::new();
        for row in page.rows {
            match row {
                p::MetadataChange::New(m) => full.push(m),
                other => mutable.push(other),
            }
        }
        // Ingest full headers first, including epoch validation and all evidence.
        // Any crash before the final transaction leaves the old modseq, so replay
        // repeats observations rather than skipping uncommitted changes.
        let mut rows = self
            .ingest_metadata_at(store, a, folder, page.uidvalidity, full, observed_after)
            .await?;
        let mut messages = Vec::new();
        let mut documents = Vec::new();
        for row in mutable {
            let (r, mut message, observation) = match row {
                p::MetadataChange::Unchanged(r) => {
                    let message = lookup.referenced(&r).await?.ok_or(ApiFailure(
                        StatusCode::BAD_GATEWAY,
                        "missing_immutable_metadata",
                    ))?;
                    (r, message, None)
                }
                p::MetadataChange::Mutable(m) => {
                    let id = provider_id(&m.identity, a)?;
                    let message = store
                        .known_provider_message(a, &id)
                        .await?
                        .ok_or(ApiFailure(
                            StatusCode::BAD_GATEWAY,
                            "missing_immutable_metadata",
                        ))?;
                    (m.reference.clone(), message, Some(m))
                }
                p::MetadataChange::New(_) => unreachable!(),
            };
            if r.account_id != a.id()
                || r.folder != folder
                || r.uidvalidity != page.uidvalidity
                || r.uid == 0
            {
                return Err(ApiFailure(
                    StatusCode::BAD_GATEWAY,
                    "invalid_provider_metadata",
                ));
            }
            if let Some(m) = observation {
                let mut folders = mailbox_memberships(folder, m.labels, probe.as_ref());
                if matches!(m.identity, p::Identity::Gmail { .. }) {
                    if let Some(probe) = &probe {
                        retain_implicit_all_mail(&mut folders, &message.folder_ids, probe);
                    }
                }
                message.folder_ids = folders;
                message.unread = !m.seen;
                message.starred = m.starred;
            } else if !message.folder_ids.iter().any(|f| f == folder) {
                // SEARCH, unlike an empty CHANGEDSINCE result, is membership
                // evidence. Another Gmail view may have updated the projection.
                message.folder_ids.push(folder.into());
            }
            let raw = serde_json::to_string(&r).map_err(Error::from)?;
            documents.push((
                format!("__content_observation:{}", message.id),
                observed_after.to_string(),
            ));
            documents.push((format!("reference:{}", message.id), raw.clone()));
            documents.push((format!("member-reference:{}:{}", message.id, folder), raw));
            documents.push((reference_key(&r), message.id.clone()));
            messages.push(message);
        }
        if let Some(checkpoint) = page.checkpoint {
            if page.next.is_some()
                || checkpoint.account_id != a.id()
                || checkpoint.folder != folder
                || checkpoint.uidvalidity != page.uidvalidity
                || checkpoint.modseq == 0
            {
                return Err(ApiFailure(
                    StatusCode::BAD_GATEWAY,
                    "invalid_provider_metadata",
                ));
            }
            documents.push((
                key,
                serde_json::to_string(&checkpoint).map_err(Error::from)?,
            ));
        }
        // All mutable changes and their high watermark share one transaction.
        // upsert_message_tx still projects pending read/star and categories.
        for message in store.ingest_messages(a, messages, &documents).await? {
            let message = store.visible_message(a, message).await?;
            if !rows.iter().any(|m| m.id == message.id)
                && (!folder.eq_ignore_ascii_case("INBOX")
                    || message
                        .folder_ids
                        .iter()
                        .any(|f| f.eq_ignore_ascii_case("INBOX")))
            {
                rows.push(message);
            }
        }
        let total = rows.len() as u32;
        let result = SearchPage {
            rows,
            continuation: page
                .next
                .map(|t| serde_json::to_vec(&t).map(|b| URL_SAFE_NO_PAD.encode(b)))
                .transpose()
                .map_err(Error::from)?,
            coverage: vec![AccountCoverage {
                account_id: a.id().into(),
                folder_ids: vec![folder.into()],
                inbox: None,
                state: if page.full_coverage {
                    CoverageState::Complete
                } else {
                    CoverageState::Partial
                },
                error: None,
            }],
            classification: ClassificationCoverage {
                classified: 0,
                total,
            },
            pending: false,
            counts_exact: false,
            cache_preview: None,
            index_page: None,
        };
        if let Some(generation) = record_generation {
            let current = store
                .scan_generation(a, folder)
                .await?
                .ok_or(Error::Conflict)?;
            // The lane covers ingestion AND checkpoint commit. Only our own
            // epoch replacement may advance the expected generation here.
            if current != generation
                && !(current == generation + 1
                    && starting_epoch != store.provider_document(a, &epoch_key).await?)
            {
                return Err(Error::Conflict.into());
            }
            store.record_scan(a, folder, current, &result).await?;
        }
        Ok(result)
    }
}
