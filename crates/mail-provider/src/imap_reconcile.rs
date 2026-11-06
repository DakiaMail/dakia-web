//! Read-only membership traversal with independent immutable acquisition.
use super::*;
use crate::provider::MetadataLookup;
use std::collections::{HashMap, HashSet};
#[path = "reconcile_timing.rs"]
mod timing;

pub(super) fn scoped_search(query: &str, token: Option<&PageToken>) -> Option<String> {
    match token {
        None => Some(query.to_owned()),
        Some(t) => {
            let upper = t.upper_uid.min(t.before_uid.saturating_sub(1));
            (upper > 0).then(|| format!("{query} UID 1:{upper}"))
        }
    }
}
fn sequence(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}
impl ImapProvider {
    /// RFC 7162 section 6: an equal persistent HIGHESTMODSEQ in the same
    /// UIDVALIDITY epoch proves a completed synchronization is still current.
    /// Never manufacture an empty snapshot from this result: it cannot prune.
    pub async fn snapshot_unchanged(&self, checkpoint: &ModseqCheckpoint) -> Result<bool> {
        if checkpoint.account_id != self.config.account_id || checkpoint.modseq == 0 {
            return Err(error(
                ErrorKind::InvalidInput,
                "reconciliation scope mismatch",
            ));
        }
        let folder = quote(&checkpoint.folder)?;
        self.bounded(async {
            let mut s = self.command().await?;
            let caps = capabilities(&mut s).await?;
            if !has(&caps, "CONDSTORE") {
                s.recycle();
                return Ok(false);
            }
            let selected = match checked(&mut s, &format!("EXAMINE {folder} (CONDSTORE)")).await {
                Ok(selected) => selected,
                Err(e) if e.kind == ErrorKind::Unsupported => return Ok(false),
                Err(e) => return Err(e),
            };
            let unchanged = !selected.no_modseq
                && selected.uidvalidity == Some(checkpoint.uidvalidity)
                && selected.highest_modseq == Some(checkpoint.modseq);
            s.recycle();
            tracing::info!(target: "dakia_server::provider_io", operation = "snapshot_check", unchanged, "provider_io");
            Ok(unchanged)
        }).await
    }
    pub async fn reconcile(
        &self,
        folder: &str,
        token: Option<&PageToken>,
        limit: usize,
        known: &dyn MetadataLookup,
        since: Option<&ModseqCheckpoint>,
    ) -> Result<ReconcilePage> {
        quote(folder)?;
        if !(1..=200).contains(&limit)
            || token.is_some_and(|t| {
                t.account_id != self.config.account_id
                    || t.folder != folder
                    || t.query != Search::default()
            })
            || since.is_some_and(|c| {
                c.account_id != self.config.account_id || c.folder != folder || c.modseq == 0
            })
        {
            return Err(error(
                ErrorKind::InvalidInput,
                "reconciliation scope mismatch",
            ));
        }
        let operation_started = std::time::Instant::now();
        let batch = self
            .reconcile_batch
            .load(std::sync::atomic::Ordering::Relaxed);
        let mut stage = "checkout";
        let result = self.bounded(async {
            let mut timing = timing::Timing::new();
            let mut s = self.command().await?;
            // Rediscover on the actual authenticated session, not the stored probe.
            stage = "capability";
            timing.stage(1);
            let caps = capabilities(&mut s).await?;
            let mut condstore = has(&caps, "CONDSTORE");
            let mut selected = None;
            stage = "select";
            timing.stage(2);
            if condstore {
                // imap-proto represents RFC 5161 ENABLED as Capabilities. Both
                // advertisement and affirmative enablement are required.
                let mut extra = " (CONDSTORE)".to_owned();
                if let Some(c) = since
                    .filter(|_| has(&caps, "QRESYNC") && has(&caps, "ENABLE") && token.is_none())
                {
                    match checked(&mut s, "ENABLE QRESYNC").await {
                        Ok(enabled) if has(&enabled.capabilities, "QRESYNC") => {
                            extra = format!(" (QRESYNC ({} {}))", c.uidvalidity, c.modseq);
                        }
                        Ok(_) => {}
                        Err(e) if e.kind == ErrorKind::Unsupported => {}
                        Err(e) => return Err(e),
                    }
                }
                match checked(&mut s, &format!("EXAMINE {}{extra}", quote(folder)?)).await {
                    Ok(output) => selected = Some(output),
                    Err(e) if e.kind == ErrorKind::Unsupported => condstore = false,
                    Err(e) => return Err(e),
                }
            }
            let selected = match selected {
                Some(output) => output,
                None => checked(&mut s, &format!("EXAMINE {}", quote(folder)?)).await?,
            };
            let validity = selected
                .uidvalidity
                .ok_or_else(|| error(ErrorKind::Unsupported, "missing UIDVALIDITY"))?;
            if token.is_some_and(|t| t.uidvalidity != validity) {
                return Err(error(
                    ErrorKind::StaleReference,
                    "UIDVALIDITY changed; restart folder scan",
                ));
            }
            let high = selected
                .highest_modseq
                .filter(|v| condstore && !selected.no_modseq && *v > 0);
            let baseline = if token.is_some() {
                token.and_then(|t| t.modseq)
            } else {
                high
            };
            // A checkpoint is usable only in this epoch, with a monotonic server
            // value. A pre-upgrade continuation cannot manufacture a baseline.
            let since = since.filter(|c| {
                c.uidvalidity == validity
                    && high.is_some_and(|v| v >= c.modseq)
                    && baseline.is_some()
            });
            stage = "search";
            timing.stage(3);
            let mut uids = match scoped_search("ALL", token) {
                Some(query) => search_uids(&mut s, &query).await?,
                None => Vec::new(),
            };
            uids.retain(|u| {
                *u != 0 && token.is_none_or(|t| *u <= t.upper_uid && *u < t.before_uid)
            });
            uids.sort_unstable_by(|a, b| b.cmp(a));
            uids.dedup();
            let upper = token.map_or_else(|| uids.first().copied().unwrap_or(0), |t| t.upper_uid);
            let total = uids.len();
            uids.truncate(limit);
            let mut rows = Vec::new();
            let mut consumed = 0;
            let mut bytes = 0;
            let started = std::time::Instant::now();
            let gmail = self.config.kind == ProviderKind::Gmail;
            // A wholly cold first chunk would otherwise pay two FETCH round
            // trips for every subsequent chunk. Prefer full metadata for the
            // remainder of this bounded page only. This is a cost heuristic,
            // never a claim that later identities are unknown or unchanged.
            let mut cold_page = false;
            for chunk in uids.chunks(batch) {
                if cold_page {
                    stage = "headers";
                    timing.stage(7);
                    let (metadata, _) = self.metadata(&mut s, folder, validity, chunk).await?;
                    bytes += metadata.iter().map(|m| m.headers.len()).sum::<usize>();
                    rows.extend(metadata.into_iter().map(MetadataChange::New));
                    consumed += chunk.len();
                    if bytes >= 2 * 1024 * 1024
                        || started.elapsed() >= std::time::Duration::from_secs(2)
                    {
                        break;
                    }
                    continue;
                }
                let reference = |uid| MessageRef {
                    account_id: self.config.account_id.clone(),
                    folder: folder.into(),
                    uidvalidity: validity,
                    uid,
                };
                let mut reusable = Vec::new();
                let mut fresh = Vec::new();
                stage = "reference_lookup";
                timing.stage(4);
                for uid in chunk {
                    if since.is_some() && known.reference_known(&reference(*uid)).await? {
                        reusable.push(*uid);
                    } else {
                        fresh.push(*uid);
                    }
                }
                let fields = if gmail {
                    "UID FLAGS X-GM-MSGID X-GM-LABELS"
                } else {
                    "UID FLAGS"
                };
                let mut observations = Vec::new();
                stage = "mutable_fetch";
                timing.stage(5);
                if !fresh.is_empty() {
                    observations.extend(fetches(&mut s, &sequence(&fresh), fields).await?);
                }
                if !reusable.is_empty() && high == since.map(|c| c.modseq) {
                    // Equal persistent MODSEQ proves no flag/label changes as of
                    // EXAMINE. SEARCH still supplies membership, and new UIDs
                    // still fetch metadata. Later changes retain the old
                    // watermark and are seen on the next reconciliation.
                    tracing::info!(target: "dakia_server::provider_io", operation = "delta_fetch_skipped", rows = reusable.len(), "provider_io");
                    rows.extend(reusable.iter().map(|uid| MetadataChange::Unchanged(reference(*uid))));
                } else if !reusable.is_empty() {
                    let command = format!(
                        "UID FETCH {} ({fields}) (CHANGEDSINCE {})",
                        sequence(&reusable),
                        since.unwrap().modseq
                    );
                    match checked(&mut s, &command).await {
                        Ok(output) => {
                            let changed: HashSet<_> =
                                output.fetches.iter().filter_map(|f| f.uid).collect();
                            for uid in &reusable {
                                if !changed.contains(uid) {
                                    // Membership comes from SEARCH, not absence in a delta.
                                    rows.push(MetadataChange::Unchanged(reference(*uid)));
                                }
                            }
                            observations.extend(output.fetches);
                        }
                        Err(e) if e.kind == ErrorKind::Unsupported => {
                            observations
                                .extend(fetches(&mut s, &sequence(&reusable), fields).await?);
                        }
                        Err(e) => return Err(e),
                    }
                }
                let mut observed = HashMap::new();
                let mut need_headers = Vec::new();
                stage = "identity_lookup";
                timing.stage(6);
                for f in observations {
                    let uid = f
                        .uid
                        .ok_or_else(|| error(ErrorKind::Retryable, "FETCH omitted UID"))?;
                    if !chunk.contains(&uid) {
                        continue;
                    }
                    if !f.has_flags || (gmail && !f.has_labels) {
                        return Err(error(ErrorKind::Retryable, "FETCH omitted mutable fields"));
                    }
                    let identity = if gmail {
                        Identity::Gmail {
                            account_id: self.config.account_id.clone(),
                            message_id: f.gmail_id.ok_or_else(|| {
                                error(ErrorKind::Unsupported, "Gmail FETCH omitted X-GM-MSGID")
                            })?,
                        }
                    } else {
                        Identity::Imap(reference(uid))
                    };
                    if observed.insert(uid, ()).is_some() {
                        return Err(error(ErrorKind::Retryable, "duplicate mutable FETCH"));
                    }
                    let draft = f
                        .labels
                        .iter()
                        .any(|v| v.trim_start_matches('\\').eq_ignore_ascii_case("Drafts"));
                    if !draft && known.identity_known(&identity).await? {
                        rows.push(MetadataChange::Mutable(MutableMetadata {
                            reference: reference(uid),
                            identity,
                            labels: f.labels,
                            seen: has(&f.flags, "\\Seen"),
                            starred: has(&f.flags, "\\Flagged"),
                        }));
                    } else {
                        need_headers.push(uid);
                    }
                }
                if !need_headers.is_empty() {
                    // Only an entire successful first chunk of missing
                    // metadata enables this optimization. Mixed/known pages
                    // and delta resumptions keep their cheaper discovery path.
                    cold_page = consumed == 0
                        && since.is_none()
                        && need_headers.len() == chunk.len();
                    stage = "headers";
                    timing.stage(7);
                    let (metadata, _) = self
                        .metadata(&mut s, folder, validity, &need_headers)
                        .await?;
                    bytes += metadata.iter().map(|m| m.headers.len()).sum::<usize>();
                    rows.extend(metadata.into_iter().map(MetadataChange::New));
                }
                consumed += chunk.len();
                if bytes >= 2 * 1024 * 1024
                    || started.elapsed() >= std::time::Duration::from_secs(2)
                {
                    break;
                }
            }
            let next = if consumed < total {
                uids.get(consumed.saturating_sub(1)).map(|uid| PageToken {
                    account_id: self.config.account_id.clone(),
                    folder: folder.into(),
                    uidvalidity: validity,
                    upper_uid: upper,
                    before_uid: *uid,
                    query: Search::default(),
                    modseq: baseline.filter(|_| high.is_some()),
                })
            } else {
                None
            };
            let checkpoint = if next.is_none() {
                baseline
                    .filter(|v| high.is_some_and(|h| h >= *v))
                    .map(|modseq| ModseqCheckpoint {
                        account_id: self.config.account_id.clone(),
                        folder: folder.into(),
                        uidvalidity: validity,
                        modseq,
                    })
            } else {
                None
            };
            tracing::info!(target: "dakia_server::provider_io", operation = "reconcile", provider = ?self.config.kind,
                rows = rows.len(), header_bytes = bytes,
                mutable_rows = rows.iter().filter(|r| matches!(r, MetadataChange::Mutable(_))).count(),
                unchanged_rows = rows.iter().filter(|r| matches!(r, MetadataChange::Unchanged(_))).count(),
                elapsed_ms = operation_started.elapsed().as_millis() as u64,
                has_next = next.is_some(), delta = since.is_some(), "provider_io");
            s.recycle();
            timing.complete = true;
            Ok(ReconcilePage {
                rows,
                next,
                uidvalidity: validity,
                full_coverage: self.coverage(),
                checkpoint,
            })
        })
        .await;
        if matches!(&result, Err(e) if e.message == "IMAP operation deadline exceeded")
            && matches!(
                stage,
                "headers" | "mutable_fetch" | "identity_lookup" | "reference_lookup"
            )
        {
            // No replay or cursor movement on failure. The next separately
            // scheduled page uses a smaller wire unit on a fresh connection.
            // Account-instance local, monotonic, bounded at one UID. Keep the
            // existing deadline/page/byte budgets rather than extending them.
            self.reconcile_batch
                .fetch_min((batch / 2).max(1), std::sync::atomic::Ordering::Relaxed);
        }
        if result.is_err() {
            // Static stages only, never raw commands, responses or mailbox IDs.
            tracing::warn!(target: "dakia_server::provider_io", operation = "reconcile_failed",
                provider = ?self.config.kind, stage,
                batch,
                elapsed_ms = operation_started.elapsed().as_millis() as u64,
                continuation = token.is_some(), "provider_io");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_search_never_sends_reversed_or_zero_range_and_preserves_query() {
        let mut token = PageToken {
            account_id: "a".into(),
            folder: "INBOX".into(),
            uidvalidity: 1,
            upper_uid: 90,
            before_uid: 70,
            query: Search::default(),
            modseq: None,
        };
        assert_eq!(
            scoped_search("ALL FLAGGED", Some(&token)).as_deref(),
            Some("ALL FLAGGED UID 1:69")
        );
        token.before_uid = 1;
        assert!(scoped_search("ALL", Some(&token)).is_none());
        token.before_uid = u32::MAX;
        assert_eq!(
            scoped_search("ALL", Some(&token)).as_deref(),
            Some("ALL UID 1:90")
        );
    }
}
