//! Gmail references may disappear while the account-wide message ID stays live.
use super::*;
impl MailRuntime {
    pub(super) async fn gmail_raw(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
        m: &Message,
        primary: p::MessageRef,
        limit: u32,
    ) -> Result<Vec<u8>> {
        let mut state = store.content_references(a, m).await?;
        let mut candidates = vec![primary];
        for raw in store.message_reference_documents(a, &m.id).await? {
            let r: p::MessageRef = serde_json::from_str(&raw).map_err(Error::from)?;
            if !candidates.contains(&r) {
                candidates.push(r)
            }
        }
        let provider = self.provider(store, a).await?;
        for reference in candidates {
            if reference.account_id != a.id() {
                continue;
            }
            let raw = serde_json::to_string(&reference).map_err(Error::from)?;
            if state.blocked.contains(&raw) {
                continue;
            }
            match provider.fetch(&reference, limit).await {
                Ok(bytes) => {
                    store
                        .promote_content_reference(a, m, &raw, &state.token)
                        .await?;
                    return Ok(bytes);
                }
                Err(e) if e.kind == p::ErrorKind::StaleReference => {
                    if !store
                        .block_content_reference(a, m, &raw, &state.token)
                        .await?
                    {
                        return Err(ApiFailure(
                            StatusCode::CONFLICT,
                            "content_reference_changed",
                        ));
                    }
                    state.blocked.push(raw.clone());
                    tracing::info!(target:"dakia_server::diagnostics",operation="content_reference_blocked",reference_key=crate::diagnostics::correlation(&raw),folder_key=crate::diagnostics::correlation(&reference.folder),reason="stale_reference", "mail_work");
                }
                Err(e) => {
                    store
                        .forget_empty_content_references(a, &m.id, &state.token)
                        .await?;
                    return Err(provider_error(e));
                }
            }
        }
        let committed = store.mark_reference_gap(a, m, &state.token).await?;
        tracing::info!(target:"dakia_server::diagnostics",operation="content_reference_gap",outcome=if committed{"unavailable"}else{"observation_changed"},"mail_work");
        Err(ApiFailure(
            StatusCode::CONFLICT,
            if committed {
                "content_reference_unavailable"
            } else {
                "content_reference_changed"
            },
        ))
    }
    #[tracing::instrument(target="dakia_server::diagnostics",skip_all,fields(account_key=crate::diagnostics::correlation(a.id()),purpose="reference_recovery"))]
    pub(crate) async fn recover_content_reference(
        &self,
        store: &Store,
        a: &AuthorizedAccount,
    ) -> Result<bool> {
        let Some(m) = store.claim_reference_recovery(a).await? else {
            return Ok(false);
        };
        let Some(id) = m
            .provider_id
            .strip_prefix("gmail:")
            .and_then(|id| id.parse::<u64>().ok())
        else {
            return Ok(true);
        };
        let mut folders = Vec::new();
        if let Some(raw) = store
            .provider_document(a, &format!("reference:{}", m.id))
            .await?
        {
            let r: p::MessageRef = serde_json::from_str(&raw).map_err(Error::from)?;
            if r.account_id == a.id() {
                folders.push(r.folder)
            }
        }
        let probe: Option<p::Probe> = store
            .provider_document(a, "probe")
            .await?
            .and_then(|s| serde_json::from_str(&s).ok());
        if let Some(probe) = probe {
            for role in ["All", "Trash", "Junk"] {
                if let Some(f) = probe.folders.iter().find(|f| {
                    f.selectable
                        && f.attributes
                            .iter()
                            .any(|r| r.trim_start_matches('\\').eq_ignore_ascii_case(role))
                }) {
                    if !folders.contains(&f.name) {
                        folders.push(f.name.clone())
                    }
                }
            }
        }
        if folders.is_empty() {
            return Ok(true);
        }
        let provider = self.provider(store, a).await?;
        let observed_after = chrono::Utc::now().timestamp_micros();
        let resolved = provider
            .resolve_gmail_message(id, &folders)
            .await
            .map_err(provider_error)?;
        if let Some(metadata) = resolved {
            if metadata.reference.account_id != a.id()
                || !folders.contains(&metadata.reference.folder)
                || !matches!(&metadata.identity,p::Identity::Gmail{account_id,message_id} if account_id==a.id()&&*message_id==id)
            {
                return Err(ApiFailure(
                    StatusCode::BAD_GATEWAY,
                    "invalid_provider_metadata",
                ));
            }
            let folder = metadata.reference.folder.clone();
            let epoch = metadata.reference.uidvalidity;
            self.ingest_metadata_at(store, a, &folder, epoch, vec![metadata], observed_after)
                .await?;
            tracing::info!(target:"dakia_server::diagnostics",message_key=crate::diagnostics::correlation(&m.id),operation="content_reference_recovery",outcome="resolved","mail_work");
        } else {
            tracing::info!(target:"dakia_server::diagnostics",message_key=crate::diagnostics::correlation(&m.id),operation="content_reference_recovery",outcome="unavailable","mail_work");
        }
        Ok(true)
    }
}
