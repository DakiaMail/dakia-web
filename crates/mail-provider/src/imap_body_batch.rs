//! Bounded read-only acquisition for small generic IMAP messages.
use super::*;
use std::collections::HashMap;

impl ImapProvider {
    pub async fn fetch_batch(
        &self,
        references: &[MessageRef],
        max_bytes: u32,
    ) -> Result<Vec<Vec<u8>>> {
        if self.config.kind != ProviderKind::Imap {
            return Err(error(
                ErrorKind::Unsupported,
                "body batch requires generic IMAP",
            ));
        }
        if references.is_empty()
            || references.len() > 50
            || max_bytes == 0
            || max_bytes > 4 * 1024 * 1024
        {
            return Err(error(ErrorKind::InvalidInput, "invalid body batch"));
        }
        let first = &references[0];
        let mut uids = Vec::new();
        for r in references {
            self.check_account(r)?;
            if r.folder != first.folder
                || r.uidvalidity != first.uidvalidity
                || uids.contains(&r.uid)
            {
                return Err(error(ErrorKind::InvalidInput, "body batch scope mismatch"));
            }
            uids.push(r.uid);
        }
        self.bounded(async {
            let mut s = self.command().await?;
            if select(&mut s, &first.folder, true).await? != first.uidvalidity {
                return Err(error(ErrorKind::StaleReference, "UIDVALIDITY changed"));
            }
            let sequence = uids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let mut sizes = HashMap::new();
            for f in fetches(&mut s, &sequence, "UID RFC822.SIZE").await? {
                let Some(uid) = f.uid.filter(|u| uids.contains(u)) else {
                    continue;
                };
                let size = f
                    .size
                    .ok_or_else(|| error(ErrorKind::Retryable, "message size unavailable"))?;
                if sizes.insert(uid, size).is_some_and(|old| old != size) {
                    return Err(error(ErrorKind::Retryable, "conflicting message sizes"));
                }
            }
            if sizes.len() != uids.len() {
                return Err(error(ErrorKind::StaleReference, "message size unavailable"));
            }
            // A single wire command retains at most 50 * 65,537 literal bytes.
            // Larger messages use the original individually bounded path.
            if sizes.values().any(|size| *size > max_bytes.min(65536)) {
                s.recycle();
                return Err(error(
                    ErrorKind::Unsupported,
                    "large body requires individual fetch",
                ));
            }
            let mut bodies = HashMap::new();
            for f in fetches(&mut s, &sequence, "UID BODY.PEEK[]<0.65537>").await? {
                let Some(uid) = f.uid.filter(|u| uids.contains(u)) else {
                    continue;
                };
                if !f.full_body || f.body_offset.unwrap_or(0) != 0 {
                    return Err(error(
                        ErrorKind::Retryable,
                        "incomplete or conflicting message body",
                    ));
                }
                let body = f
                    .body
                    .ok_or_else(|| error(ErrorKind::StaleReference, "message body unavailable"))?;
                if body.len() != sizes[&uid] as usize || body.len() > 65536 {
                    return Err(error(
                        ErrorKind::Retryable,
                        "incomplete or conflicting message body",
                    ));
                }
                if bodies.get(&uid).is_some_and(|old| old != &body) {
                    return Err(error(
                        ErrorKind::Retryable,
                        "incomplete or conflicting message body",
                    ));
                }
                bodies.insert(uid, body);
            }
            if bodies.len() != uids.len() {
                return Err(error(ErrorKind::StaleReference, "message body unavailable"));
            }
            let rows = uids.iter().map(|uid| bodies.remove(uid).unwrap()).collect();
            s.recycle();
            Ok(rows)
        })
        .await
    }
}
