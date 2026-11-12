use super::*;

impl ImapProvider {
    pub async fn resolve_gmail_message(
        &self,
        message_id: u64,
        folders: &[String],
    ) -> Result<Option<Metadata>> {
        if self.config.kind != ProviderKind::Gmail
            || message_id == 0
            || folders.is_empty()
            || folders.len() > 4
        {
            return Err(error(
                ErrorKind::InvalidInput,
                "invalid Gmail identity recovery scope",
            ));
        }
        for f in folders {
            quote(f)?;
        }
        self.bounded(async {
            let mut s = self.command().await?;
            let caps = capabilities(&mut s).await?;
            if !has(&caps, "X-GM-EXT-1") {
                return Err(error(
                    ErrorKind::Unsupported,
                    "Gmail identity recovery unsupported",
                ));
            }
            for folder in folders {
                let epoch = match select(&mut s, folder, true).await {
                    Ok(epoch) => epoch,
                    Err(e)
                        if e.kind == ErrorKind::Unsupported
                            && e.message == "IMAP command rejected" =>
                    {
                        continue
                    }
                    Err(e) => return Err(e),
                };
                let uids = search_uids(&mut s, &format!("X-GM-MSGID {message_id}")).await?;
                if uids.is_empty() {
                    continue;
                }
                if uids.len() != 1 || uids[0] == 0 {
                    return Err(error(
                        ErrorKind::Retryable,
                        "ambiguous Gmail identity recovery",
                    ));
                }
                let (mut rows, _) = self.metadata(&mut s, folder, epoch, &uids).await?;
                let Some(row) = rows.pop() else { continue };
                if !rows.is_empty()
                    || row.identity
                        != (Identity::Gmail {
                            account_id: self.config.account_id.clone(),
                            message_id,
                        })
                {
                    return Err(error(
                        ErrorKind::Retryable,
                        "Gmail identity recovery mismatch",
                    ));
                }
                s.recycle();
                return Ok(Some(row));
            }
            s.recycle();
            Ok(None)
        })
        .await
    }

    /// Check only arrivals above a committed watermark. On first use or epoch
    /// reset, seed from the newest 50; historical reconciliation remains separate.
    pub async fn head(&self, folder: &str, after: Option<&HeadCheckpoint>) -> Result<HeadPage> {
        quote(folder)?;
        if after.is_some_and(|c| c.account_id != self.config.account_id || c.folder != folder) {
            return Err(error(
                ErrorKind::InvalidInput,
                "head checkpoint scope mismatch",
            ));
        }
        self.bounded(async {
            let mut s = self.command().await?;
            let validity = select(&mut s, folder, true).await?;
            let after = after.filter(|c| c.uidvalidity == validity);
            // IMAP reverses a range whose lower bound exceeds '*'. Filter locally
            // too; never send MAX+1 or interpret the last old UID as an arrival.
            let mut uids = match after {
                Some(c) if c.uid == u32::MAX => Vec::new(),
                Some(c) => search_uids(&mut s, &format!("UID {}:*", c.uid + 1)).await?,
                None => search_uids(&mut s, "ALL").await?,
            };
            uids.retain(|u| *u != 0 && after.is_none_or(|c| *u > c.uid));
            uids.sort_unstable();
            uids.dedup();
            // Ascending incremental pages ensure a burst larger than 50 is not
            // skipped. Only the bootstrap takes newest-first historical rows.
            if after.is_none() {
                uids.reverse();
            }
            uids.truncate(50);
            let (rows, consumed) = self.metadata(&mut s, folder, validity, &uids).await?;
            let uid = if after.is_none() {
                uids.first().copied().unwrap_or(0)
            } else {
                uids.get(consumed.saturating_sub(1))
                    .copied()
                    .unwrap_or_else(|| after.unwrap().uid)
            };
            tracing::info!(target: "dakia_server::provider_io", operation = "head", rows = rows.len(), "provider_io");
            s.recycle();
            Ok(HeadPage {
                rows,
                checkpoint: HeadCheckpoint {
                    account_id: self.config.account_id.clone(),
                    folder: folder.into(),
                    uidvalidity: validity,
                    uid,
                },
            })
        })
        .await
    }

    /// Fetch in small wire units, stopping after 2 MiB of accepted headers.
    /// At most 50 additional <64 KiB headers can cross that soft boundary, so
    /// retained page headers stay below 6 MiB. The caller resumes at the last
    /// attempted UID (including expunged UIDs), not at the last returned row.
    pub(super) async fn metadata(
        &self,
        s: &mut Connection,
        folder: &str,
        validity: u32,
        uids: &[u32],
    ) -> Result<(Vec<Metadata>, usize)> {
        let gmail = self.config.kind == ProviderKind::Gmail;
        let fields = if gmail {
            "UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID X-GM-THRID X-GM-LABELS BODY.PEEK[HEADER]<0.65536>"
        } else {
            "UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER]<0.65536>"
        };
        let mut rows = Vec::new();
        let mut consumed = 0;
        let mut bytes = 0;
        let started = std::time::Instant::now();
        for chunk in uids.chunks(50) {
            let sequence = chunk
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            for f in fetches(s, &sequence, fields).await? {
                let uid = f
                    .uid
                    .ok_or_else(|| error(ErrorKind::Retryable, "FETCH omitted UID"))?;
                if !chunk.contains(&uid) {
                    continue;
                }
                if !f.has_flags || (gmail && !f.has_labels) {
                    return Err(error(ErrorKind::Retryable, "FETCH omitted mutable fields"));
                }
                if rows.iter().any(|r: &Metadata| r.reference.uid == uid) {
                    return Err(error(ErrorKind::Retryable, "duplicate metadata FETCH"));
                }
                let reference = MessageRef {
                    graph_id: None,
                    account_id: self.config.account_id.clone(),
                    folder: folder.into(),
                    uidvalidity: validity,
                    uid,
                };
                let identity = if gmail {
                    Identity::Gmail {
                        account_id: self.config.account_id.clone(),
                        message_id: f.gmail_id.ok_or_else(|| {
                            error(ErrorKind::Unsupported, "Gmail FETCH omitted X-GM-MSGID")
                        })?,
                    }
                } else {
                    Identity::Imap(reference.clone())
                };
                if gmail && f.gmail_thread.is_none() {
                    return Err(error(
                        ErrorKind::Unsupported,
                        "Gmail FETCH omitted X-GM-THRID",
                    ));
                }
                let headers = f
                    .body
                    .ok_or_else(|| error(ErrorKind::Retryable, "FETCH omitted headers"))?;
                if headers.len() >= 65536 {
                    return Err(error(
                        ErrorKind::Unsupported,
                        "message headers exceed metadata limit",
                    ));
                }
                bytes += headers.len();
                let internal_date = f
                    .internal_date
                    .ok_or_else(|| error(ErrorKind::Retryable, "FETCH omitted INTERNALDATE"))?;
                rows.push(Metadata {
                    reference,
                    identity,
                    gmail_thread_id: f.gmail_thread,
                    labels: f.labels,
                    seen: has(&f.flags, "\\Seen"),
                    starred: has(&f.flags, "\\Flagged"),
                    size: f.size.unwrap_or(0),
                    internal_date,
                    headers,
                });
            }
            consumed += chunk.len();
            if bytes >= 2 * 1024 * 1024 || started.elapsed() >= std::time::Duration::from_secs(2) {
                break;
            }
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.reference.uid));
        Ok((rows, consumed))
    }
}
