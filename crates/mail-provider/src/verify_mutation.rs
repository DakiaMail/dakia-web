//! Readback is separate from a write, on a clean connection after uncertainty.
//! Absence alone never proves a move: Gmail recovery requires stable identity
//! in the intended destination as well as loss of the source membership.
use super::*;
impl ImapProvider {
    pub async fn verify_mutation(
        &self,
        r: &MessageRef,
        identity: &Identity,
        change: &Mutation,
    ) -> Result<Option<MutationResult>> {
        self.check_account(r)?;
        match identity {
            Identity::Imap(reference) if reference != r => {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "mutation verification scope mismatch",
                ))
            }
            Identity::Gmail { account_id, .. } if account_id != &self.config.account_id => {
                return Err(error(
                    ErrorKind::InvalidInput,
                    "mutation verification scope mismatch",
                ))
            }
            _ => {}
        }
        // No generic move/copy recovery without stable cross-folder identity.
        let gmail_archive = matches!(
            (identity, change),
            (Identity::Gmail { .. }, Mutation::Archive)
        ) && self.config.kind == ProviderKind::Gmail
            && r.folder.eq_ignore_ascii_case("INBOX");
        if !gmail_archive && !matches!(change, Mutation::Seen(_) | Mutation::Starred(_)) {
            return Ok(None);
        }
        self.bounded(async {
            let mut s = self.command().await?;
            let selected = checked(&mut s, &format!("EXAMINE {}", quote(&r.folder)?)).await?;
            if selected.uidvalidity != Some(r.uidvalidity) {
                return Err(error(
                    ErrorKind::StaleReference,
                    "UIDVALIDITY changed during mutation verification",
                ));
            }
            if let Mutation::Seen(value) | Mutation::Starred(value) = change {
                let rows = fetches(&mut s, &r.uid.to_string(), "UID FLAGS").await?;
                let flag = if matches!(change, Mutation::Seen(_)) {
                    "\\Seen"
                } else {
                    "\\Flagged"
                };
                let verified = rows.len() == 1
                    && rows[0].uid == Some(r.uid)
                    && rows[0].has_flags
                    && has(&rows[0].flags, flag) == *value;
                s.recycle();
                return Ok(verified.then_some(MutationResult {
                    destination: None,
                    destination_reference: None,
                    requires_rescan: false,
                }));
            }
            let dest = self.special_folder(&mut s, "Archive").await?;
            if dest.eq_ignore_ascii_case(&r.folder) {
                return Ok(None);
            }
            if search_uids(&mut s, &format!("UID {}", r.uid))
                .await?
                .contains(&r.uid)
            {
                s.recycle();
                return Ok(None);
            }
            let Identity::Gmail { message_id, .. } = identity else {
                unreachable!()
            };
            let selected = checked(&mut s, &format!("EXAMINE {}", quote(&dest)?)).await?;
            let validity = selected
                .uidvalidity
                .ok_or_else(|| error(ErrorKind::Unsupported, "missing UIDVALIDITY"))?;
            let uids = search_uids(&mut s, &format!("X-GM-MSGID {message_id}")).await?;
            if uids.len() != 1 || uids[0] == 0 {
                s.recycle();
                return Ok(None);
            }
            let destination_reference = MessageRef {
                graph_id: None,
                account_id: r.account_id.clone(),
                folder: dest.clone(),
                uidvalidity: validity,
                uid: uids[0],
            };
            s.recycle();
            Ok(Some(MutationResult {
                destination: Some(dest),
                destination_reference: Some(destination_reference),
                requires_rescan: true,
            }))
        })
        .await
    }
}
