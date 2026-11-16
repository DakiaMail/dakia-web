use super::*;

impl MailboxTransaction<'_> {
    async fn content_references(&mut self, message: &Message) -> Result<ContentReferences> {
        if message.account_id != self.account || !message.provider_id.starts_with("gmail:") {
            return Err(Error::Invalid);
        }
        let initial = ContentReferences {
            token: Uuid::new_v4().to_string(),
            source: message.provider_id.clone(),
            blocked: vec![],
            blocked_at: 0,
        };
        let key = format!("content:references:{}", message.id);
        let raw = serde_json::to_string(&initial)?;
        write!(self,"INSERT INTO provider_documents(key,document) SELECT ?,? WHERE EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE json_extract(document,'$.source')<>json_extract(excluded.document,'$.source')",[&key,&raw,&message.id,&message.provider_id],"INSERT INTO provider_documents(account_id,key,document) SELECT $1,$2,$3 WHERE EXISTS(SELECT 1 FROM messages WHERE account_id=$1 AND id=$4 AND source=$5) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document WHERE provider_documents.document::jsonb->>'source'<>excluded.document::jsonb->>'source'",[&self.account,&key,&raw,&message.id,&message.provider_id]);
        let raw: String = value!(
            self,
            "SELECT document FROM provider_documents WHERE key=?",
            [&key],
            "SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2",
            [&self.account, &key],
            fetch_one
        );
        let state: ContentReferences = serde_json::from_str(&raw)?;
        if state.source != message.provider_id {
            return Err(Error::Conflict);
        }
        Ok(state)
    }
    async fn block_content_reference(
        &mut self,
        message: &Message,
        reference: &str,
        token: &str,
    ) -> Result<bool> {
        if message.account_id != self.account {
            return Err(Error::Invalid);
        }
        let key = format!("content:references:{}", message.id);
        write!(
            self,
            "UPDATE provider_documents SET document=document WHERE key=?",
            [&key],
            "UPDATE provider_documents SET document=document WHERE account_id=$1 AND key=$2",
            [&self.account, &key]
        );
        let raw:Option<String>=value!(self,"SELECT document FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_extract(document,'$.source')=? AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=?)",[&key,token,&message.provider_id,&message.id,&message.provider_id],"SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2 AND document::jsonb->>'token'=$3 AND document::jsonb->>'source'=$4 AND EXISTS(SELECT 1 FROM messages WHERE account_id=$1 AND id=$5 AND source=$4)",[&self.account,&key,token,&message.provider_id,&message.id],fetch_optional);
        let Some(raw) = raw else {
            return Ok(false);
        };
        let mut state: ContentReferences = serde_json::from_str(&raw)?;
        if !state.blocked.iter().any(|r| r == reference) {
            if state.blocked.len() >= 257 {
                return Err(Error::Invalid);
            }
            state.blocked.push(reference.into());
            state.blocked_at = Utc::now().timestamp_micros();
            let raw = serde_json::to_string(&state)?;
            write!(
                self,
                "UPDATE provider_documents SET document=? WHERE key=?",
                [&raw, &key],
                "UPDATE provider_documents SET document=$1 WHERE account_id=$2 AND key=$3",
                [&raw, &self.account, &key]
            );
        }
        Ok(true)
    }
    async fn mark_reference_gap(&mut self, message: &Message, token: &str) -> Result<bool> {
        if message.account_id != self.account {
            return Err(Error::Invalid);
        }
        let at = Utc::now().timestamp();
        let key = format!("content:references:{}", message.id);
        let reference = format!("reference:{}", message.id);
        let changed=write!(self,"UPDATE search_documents SET body_state='reference_unavailable',retry_at=CASE WHEN body_state='reference_unavailable' THEN retry_at ELSE 0 END,updated_at=? WHERE message_id=? AND source=? AND body_state IN ('pending','fetch_error','reference_unavailable') AND EXISTS(SELECT 1 FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_extract(document,'$.source')=? AND json_array_length(document,'$.blocked')>0) AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=?) AND EXISTS(SELECT 1 FROM provider_documents p,json_each((SELECT document FROM provider_documents WHERE key=?),'$.blocked') b WHERE p.key=? AND p.document=b.value)",[at,&message.id,&message.provider_id,&key,token,&message.provider_id,&message.id,&message.provider_id,&key,&reference],"UPDATE search_documents SET body_state='reference_unavailable',retry_at=CASE WHEN body_state='reference_unavailable' THEN retry_at ELSE 0 END,updated_at=$1 WHERE account_id=$2 AND message_id=$3 AND source=$4 AND body_state IN ('pending','fetch_error','reference_unavailable') AND EXISTS(SELECT 1 FROM provider_documents WHERE account_id=$2 AND key=$5 AND document::jsonb->>'token'=$6 AND document::jsonb->>'source'=$4 AND jsonb_array_length(document::jsonb->'blocked')>0) AND EXISTS(SELECT 1 FROM messages WHERE account_id=$2 AND id=$3 AND source=$4) AND EXISTS(SELECT 1 FROM provider_documents p CROSS JOIN LATERAL jsonb_array_elements_text((SELECT document::jsonb->'blocked' FROM provider_documents WHERE account_id=$2 AND key=$5)) blocked WHERE p.account_id=$2 AND p.key=$7 AND p.document=blocked)",[at,&self.account,&message.id,&message.provider_id,&key,token,&reference]);
        Ok(changed > 0)
    }
    async fn promote_content_reference(
        &mut self,
        message: &Message,
        reference: &str,
        token: &str,
    ) -> Result<()> {
        if message.account_id != self.account {
            return Err(Error::Invalid);
        }
        let key = format!("content:references:{}", message.id);
        write!(
            self,
            "UPDATE provider_documents SET document=document WHERE key=?",
            [&key],
            "UPDATE provider_documents SET document=document WHERE account_id=$1 AND key=$2",
            [&self.account, &key]
        );
        let raw:Option<String>=value!(self,"SELECT document FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_extract(document,'$.source')=?",[&key,token,&message.provider_id],"SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2 AND document::jsonb->>'token'=$3 AND document::jsonb->>'source'=$4",[&self.account,&key,token,&message.provider_id],fetch_optional);
        if let Some(raw) = raw {
            let mut state: ContentReferences = serde_json::from_str(&raw)?;
            state.blocked.retain(|r| r != reference);
            state.token = Uuid::new_v4().to_string();
            let reference_key = format!("reference:{}", message.id);
            write!(
                self,
                "UPDATE provider_documents SET document=? WHERE key=?",
                [reference, &reference_key],
                "UPDATE provider_documents SET document=$1 WHERE account_id=$2 AND key=$3",
                [reference, &self.account, &reference_key]
            );
            if state.blocked.is_empty() {
                write!(
                    self,
                    "DELETE FROM provider_documents WHERE key=?",
                    [&key],
                    "DELETE FROM provider_documents WHERE account_id=$1 AND key=$2",
                    [&self.account, &key]
                );
            } else {
                let raw = serde_json::to_string(&state)?;
                write!(
                    self,
                    "UPDATE provider_documents SET document=? WHERE key=?",
                    [&raw, &key],
                    "UPDATE provider_documents SET document=$1 WHERE account_id=$2 AND key=$3",
                    [&raw, &self.account, &key]
                );
            }
        }
        Ok(())
    }
    async fn claim_reference_recovery(&mut self) -> Result<Option<Message>> {
        let at = Utc::now().timestamp();
        let next = (at + 300).to_string();
        let admitted=write!(self,"INSERT INTO provider_documents(key,document) VALUES('content:recovery_due',?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE CAST(document AS INTEGER)<=?",[&next,at],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,'content:recovery_due',$2) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document WHERE provider_documents.document::BIGINT<=$3",[&self.account,&next,at]);
        if admitted == 0 {
            return Ok(None);
        }
        let raw:Option<String>=value!(self,"SELECT m.document FROM search_documents d JOIN messages m ON m.id=d.message_id WHERE d.body_state='reference_unavailable' AND d.retry_at<=? AND d.source=json_extract(m.document,'$.provider_id') AND json_array_length(m.document,'$.folder_ids')>0 ORDER BY d.retry_at,d.message_id LIMIT 1",[at],"SELECT m.document FROM search_documents d JOIN messages m ON m.account_id=d.account_id AND m.id=d.message_id WHERE d.account_id=$1 AND d.body_state='reference_unavailable' AND d.retry_at<=$2 AND d.source=m.source AND cardinality(m.folder_ids)>0 ORDER BY d.retry_at,d.message_id LIMIT 1",[&self.account,at],fetch_optional);
        let Some(raw) = raw else {
            return Ok(None);
        };
        let message: Message = serde_json::from_str(&raw)?;
        write!(self,"UPDATE search_documents SET retry_at=? WHERE message_id=? AND source=? AND body_state='reference_unavailable'",[at+86400,&message.id,&message.provider_id],"UPDATE search_documents SET retry_at=$1 WHERE account_id=$2 AND message_id=$3 AND source=$4 AND body_state='reference_unavailable'",[at+86400,&self.account,&message.id,&message.provider_id]);
        Ok(Some(message))
    }
}
impl AccountRepository {
    pub(crate) async fn content_references(&self, message: &Message) -> Result<ContentReferences> {
        scoped!(self, scope, true, {
            scope.content_references(message).await
        })
    }
    pub(crate) async fn block_content_reference(
        &self,
        message: &Message,
        reference: &str,
        token: &str,
    ) -> Result<bool> {
        scoped!(self, scope, true, {
            scope
                .block_content_reference(message, reference, token)
                .await
        })
    }
    pub(crate) async fn mark_reference_gap(&self, message: &Message, token: &str) -> Result<bool> {
        scoped!(self, scope, true, {
            scope.mark_reference_gap(message, token).await
        })
    }
    pub(crate) async fn promote_content_reference(
        &self,
        message: &Message,
        reference: &str,
        token: &str,
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope
                .promote_content_reference(message, reference, token)
                .await
        })
    }
    pub(crate) async fn forget_empty_content_references(
        &self,
        id: &str,
        token: &str,
    ) -> Result<()> {
        let key = format!("content:references:{id}");
        transaction!(self, tx, local, true, {
            execute!(local,tx,"DELETE FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_array_length(document,'$.blocked')=0",[&key,token],"DELETE FROM provider_documents WHERE account_id=$1 AND key=$2 AND document::jsonb->>'token'=$3 AND jsonb_array_length(document::jsonb->'blocked')=0",[&self.account,&key,token]);
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn claim_reference_recovery(&self) -> Result<Option<Message>> {
        // An empty poll must not delay a gap that arrives immediately afterward.
        // Commit admission only when a candidate was actually selected.
        match &self.backend {
            Backend::Sqlite(db) => {
                let mut tx = db.begin_with("BEGIN IMMEDIATE").await?;
                let mut scope = MailboxTransaction::sqlite(&mut tx, &self.account);
                let result = scope.claim_reference_recovery().await?;
                drop(scope);
                if result.is_some() {
                    tx.commit().await?;
                }
                Ok(result)
            }
            Backend::Postgres(db) => {
                let mut tx = self.begin_postgres(db, true).await?;
                let mut scope = MailboxTransaction::postgres(&mut tx, &self.account);
                let result = scope.claim_reference_recovery().await?;
                drop(scope);
                if result.is_some() {
                    tx.commit().await?;
                }
                Ok(result)
            }
        }
    }
}
