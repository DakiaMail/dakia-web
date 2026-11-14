//! Source acquisition and current-state checks shared by both application backends.
use super::*;
type SourceInventoryRow = (
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<String>,
    Option<String>,
    i64,
    i64,
    i64,
);
use crate::full_text::{normalize_search, SearchContent};
impl MailboxTransaction<'_> {
    async fn index_search_headers(&mut self) -> Result<()> {
        let excluded: Vec<String> = value!(
            self,
            "SELECT lower(name) FROM search_excluded_folders",
            [],
            "SELECT lower(name) FROM search_excluded_folders WHERE account_id=$1",
            [&self.account],
            fetch_all
        );
        let rows:Vec<String>=value!(self,"SELECT m.document FROM search_dirty d JOIN messages m ON m.id=d.message_id ORDER BY d.message_id LIMIT 200",[],"SELECT m.document FROM search_dirty d JOIN messages m ON m.account_id=d.account_id AND m.id=d.message_id WHERE d.account_id=$1 ORDER BY d.message_id COLLATE \"C\" LIMIT 200",[&self.account],fetch_all);
        for raw in rows {
            let m: Message = serde_json::from_str(&raw)?;
            let live:bool=value!(self,"SELECT CASE WHEN json_valid(?) THEN NOT EXISTS(SELECT 1 FROM provider_documents WHERE key='thread:epoch:'||json_extract(?,'$.folder') AND CAST(document AS INTEGER)<>json_extract(?,'$.uidvalidity')) ELSE 1 END",[&m.provider_id,&m.provider_id,&m.provider_id],"SELECT NOT EXISTS(SELECT 1 FROM messages m JOIN provider_documents v ON v.account_id=m.account_id AND v.key='thread:epoch:'||m.provider_folder WHERE m.account_id=$1 AND m.id=$2 AND v.document::BIGINT<>m.provider_epoch)",[&self.account,&m.id],fetch_one);
            if !m.folder_ids.is_empty()
                && live
                && !m
                    .folder_ids
                    .iter()
                    .any(|f| excluded.contains(&f.to_lowercase()))
            {
                let address =
                    |v: &Address| format!("{} {}", v.email, v.name.as_deref().unwrap_or(""));
                let sender = normalize_search(&address(&m.sender));
                let recipients =
                    normalize_search(&m.to.iter().map(address).collect::<Vec<_>>().join("\n"));
                let subject = normalize_search(&m.subject);
                write!(self,"INSERT INTO search_documents(message_id,source,sender,recipients,subject,received_at) VALUES(?,?,?,?,?,?) ON CONFLICT(message_id) DO UPDATE SET source=excluded.source,sender=excluded.sender,recipients=excluded.recipients,subject=excluded.subject,received_at=excluded.received_at,body=CASE WHEN source=excluded.source THEN body ELSE '' END,participants=CASE WHEN source=excluded.source THEN participants ELSE '' END,mime_recipients=CASE WHEN source=excluded.source THEN mime_recipients ELSE '' END,attachment_text=CASE WHEN source=excluded.source THEN attachment_text ELSE '' END,attachment_details=CASE WHEN source=excluded.source THEN attachment_details ELSE '[]' END,body_payload=CASE WHEN source=excluded.source THEN body_payload ELSE 0 END,body_state=CASE WHEN source=excluded.source THEN body_state ELSE 'pending' END,retry_at=CASE WHEN source=excluded.source THEN retry_at ELSE 0 END WHERE source<>excluded.source OR sender<>excluded.sender OR recipients<>excluded.recipients OR subject<>excluded.subject OR received_at<>excluded.received_at",[&m.id,&m.provider_id,&sender,&recipients,&subject,&m.received_at],"INSERT INTO search_documents(account_id,message_id,source,sender,recipients,subject,received_at) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(account_id,message_id) DO UPDATE SET source=excluded.source,sender=excluded.sender,recipients=excluded.recipients,subject=excluded.subject,received_at=excluded.received_at,body=CASE WHEN search_documents.source=excluded.source THEN search_documents.body ELSE '' END,participants=CASE WHEN search_documents.source=excluded.source THEN search_documents.participants ELSE '' END,mime_recipients=CASE WHEN search_documents.source=excluded.source THEN search_documents.mime_recipients ELSE '' END,attachment_text=CASE WHEN search_documents.source=excluded.source THEN search_documents.attachment_text ELSE '' END,attachment_details=CASE WHEN search_documents.source=excluded.source THEN search_documents.attachment_details ELSE '[]' END,body_payload=CASE WHEN search_documents.source=excluded.source THEN search_documents.body_payload ELSE 0 END,body_state=CASE WHEN search_documents.source=excluded.source THEN search_documents.body_state ELSE 'pending' END,retry_at=CASE WHEN search_documents.source=excluded.source THEN search_documents.retry_at ELSE 0 END WHERE search_documents.source<>excluded.source OR search_documents.sender<>excluded.sender OR search_documents.recipients<>excluded.recipients OR search_documents.subject<>excluded.subject OR search_documents.received_at<>excluded.received_at",[&self.account,&m.id,&m.provider_id,&sender,&recipients,&subject,&m.received_at]);
            } else {
                write!(
                    self,
                    "DELETE FROM search_documents WHERE message_id=?",
                    [&m.id],
                    "DELETE FROM search_documents WHERE account_id=$1 AND message_id=$2",
                    [&self.account, &m.id]
                );
            }
            write!(
                self,
                "DELETE FROM search_dirty WHERE message_id=?",
                [&m.id],
                "DELETE FROM search_dirty WHERE account_id=$1 AND message_id=$2",
                [&self.account, &m.id]
            );
        }
        Ok(())
    }
    async fn put_search_records(&mut self, records: &[SearchContent<'_>]) -> Result<()> {
        for record in records {
            let (m, text, state) = (record.message, normalize_search(record.text), record.state);
            let state = if text.len() > 1024 * 1024 {
                "oversized"
            } else if state == "quota" {
                "pending"
            } else {
                state
            };
            let body = if matches!(state, "ready" | "partial" | "oversized")
                && text.len() <= 1024 * 1024
            {
                text.as_str()
            } else {
                ""
            };
            let retry = if state == "fetch_error" {
                Utc::now().timestamp() + 3600
            } else {
                0
            };
            let updated = Utc::now().timestamp();
            let participants = normalize_search(record.participants);
            let recipients = normalize_search(record.recipients);
            let attachments = normalize_search(record.attachments);
            write!(self,"UPDATE search_documents SET body_payload=1,body=?,body_state=?,retry_at=?,updated_at=?,participants=?,mime_recipients=?,attachment_text=?,attachment_details=? WHERE message_id=? AND source=? AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_array_length(document,'$.folder_ids')>0 AND json_extract(document,'$.provider_id')=?)",[body,state,retry,updated,&participants,&recipients,&attachments,record.attachment_details,&m.id,&m.provider_id,&m.id,&m.provider_id],"UPDATE search_documents d SET body_payload=1,body=$1,body_state=$2,retry_at=$3,updated_at=$4,participants=$5,mime_recipients=$6,attachment_text=$7,attachment_details=$8 WHERE d.account_id=$9 AND d.message_id=$10 AND d.source=$11 AND EXISTS(SELECT 1 FROM messages m WHERE m.account_id=d.account_id AND m.id=d.message_id AND cardinality(m.folder_ids)>0 AND m.source=$11)",[body,state,retry,updated,&participants,&recipients,&attachments,record.attachment_details,&self.account,&m.id,&m.provider_id]);
        }
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn index_search_headers(&self) -> Result<()> {
        scoped!(self, scope, true, { scope.index_search_headers().await })
    }
    pub(crate) async fn put_search_records(&self, records: &[SearchContent<'_>]) -> Result<()> {
        let rows: Vec<_> = records
            .iter()
            .map(|r| (r.message, r.text, r.state))
            .collect();
        if records.iter().any(|r| {
            r.participants.len() + r.recipients.len() > 1024 * 1024
                || r.attachments.len() > 16 * 1024 * 1024
                || r.attachment_details.len() > 1024 * 1024
                || serde_json::from_str::<serde_json::Value>(r.attachment_details).is_err()
        }) {
            return Err(Error::Invalid);
        }
        if rows.len() > 200
            || rows.iter().map(|(_, text, _)| text.len()).sum::<usize>() > 8 * 1024 * 1024
            || rows.iter().any(|(m, _, state)| {
                m.account_id != self.account
                    || !matches!(
                        *state,
                        "ready"
                            | "partial"
                            | "encrypted"
                            | "oversized"
                            | "parse_error"
                            | "fetch_error"
                            | "quota"
                    )
            })
        {
            return Err(Error::Invalid);
        }
        scoped!(self, scope, true, {
            scope.put_search_records(records).await
        })
    }
    pub(crate) async fn next_search_bodies(
        &self,
        oldest: bool,
        limit: usize,
    ) -> Result<Vec<Message>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::Invalid);
        }
        transaction!(self, tx, local, false, {
            let order = if oldest {
                "d.message_id"
            } else {
                "json_extract(m.document,'$.received_at') DESC,d.message_id"
            };
            let pg_order = if oldest {
                "d.message_id COLLATE \"C\""
            } else {
                "m.received_at DESC,d.message_id COLLATE \"C\""
            };
            let rows:Vec<String>=scalar!(local,tx,&format!("SELECT m.document FROM search_documents d JOIN messages m ON m.id=d.message_id WHERE d.body_state IN ('pending','fetch_error','quota') AND d.retry_at<=? AND json_array_length(m.document,'$.folder_ids')>0 ORDER BY {order} LIMIT ?"),[Utc::now().timestamp(),limit as i64],&format!("SELECT m.document FROM search_documents d JOIN messages m ON m.account_id=d.account_id AND m.id=d.message_id WHERE d.account_id=$1 AND d.body_state IN ('pending','fetch_error','quota') AND d.retry_at<=$2 AND cardinality(m.folder_ids)>0 ORDER BY {pg_order} LIMIT $3"),[&self.account,Utc::now().timestamp(),limit as i64],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn search_scans_complete(&self, folders: &[String]) -> Result<bool> {
        if folders.is_empty() {
            return Ok(false);
        }
        let raw = serde_json::to_string(folders)?;
        transaction!(self, tx, local, false, {
            // Completion belongs to the current saved generation, not to a time
            // window. A known epoch must match even if one side is missing.
            let count:i64=scalar!(local,tx,"SELECT COUNT(*) FROM folder_scans f LEFT JOIN provider_documents e ON e.key='thread:epoch:'||f.folder WHERE f.folder IN (SELECT value FROM json_each(?)) AND f.completed_at IS NOT NULL AND f.coverage_limited=0 AND f.continuation IS NULL AND f.uidvalidity IS CAST(e.document AS INTEGER)",[&raw],"SELECT COUNT(*) FROM folder_scans f LEFT JOIN provider_documents e ON e.account_id=f.account_id AND e.key='thread:epoch:'||f.folder WHERE f.account_id=$1 AND f.folder IN (SELECT jsonb_array_elements_text($2::jsonb)) AND f.completed_at IS NOT NULL AND NOT f.coverage_limited AND f.continuation IS NULL AND f.uidvalidity IS NOT DISTINCT FROM e.document::BIGINT",[&self.account,&raw],fetch_one);
            Ok(count == folders.len() as i64)
        })
    }
}
impl MailboxTransaction<'_> {
    async fn search_excluded_folders(&mut self) -> Result<Vec<String>> {
        Ok(value!(self,"SELECT name FROM search_excluded_folders ORDER BY name",[],"SELECT name FROM search_excluded_folders WHERE account_id=$1 ORDER BY name COLLATE \"C\"",[&self.account],fetch_all))
    }
    async fn set_search_excluded_folders(&mut self, folders: &[String]) -> Result<()> {
        let mut names = vec![
            "Trash".to_owned(),
            "Spam".into(),
            "Junk".into(),
            "[Gmail]/Trash".into(),
            "[Gmail]/Spam".into(),
        ];
        names.extend(folders.iter().cloned());
        names.sort();
        names.dedup();
        if self.search_excluded_folders().await? == names {
            return Ok(());
        }
        write!(
            self,
            "DELETE FROM search_excluded_folders",
            [],
            "DELETE FROM search_excluded_folders WHERE account_id=$1",
            [&self.account]
        );
        for name in names {
            write!(
                self,
                "INSERT INTO search_excluded_folders(name) VALUES(?)",
                [&name],
                "INSERT INTO search_excluded_folders(account_id,name) VALUES($1,$2)",
                [&self.account, &name]
            );
        }
        write!(self,"INSERT OR IGNORE INTO search_dirty SELECT id FROM messages",[],"INSERT INTO search_dirty(account_id,message_id) SELECT account_id,id FROM messages WHERE account_id=$1 ON CONFLICT DO NOTHING",[&self.account]);
        write!(self,"DELETE FROM search_documents WHERE message_id IN (SELECT m.id FROM messages m,json_each(m.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name))",[],"DELETE FROM search_documents d USING messages m WHERE d.account_id=$1 AND m.account_id=d.account_id AND m.id=d.message_id AND EXISTS(SELECT 1 FROM unnest(m.folder_ids) f JOIN search_excluded_folders e ON e.account_id=m.account_id AND lower(f)=lower(e.name))",[&self.account]);
        write!(self,"UPDATE search_totals SET eligible=(SELECT count(*) FROM search_eligible_messages) WHERE id=1",[],"UPDATE search_inventory i SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=i.account_id AND e.id=i.message_id) WHERE i.account_id=$1",[&self.account]);
        write!(self,"UPDATE search_cleanup_work SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.id=search_cleanup_work.message_id)",[],"UPDATE search_cleanup_work w SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=w.account_id AND e.id=w.message_id) WHERE w.account_id=$1",[&self.account]);
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn search_excluded_folders(&self) -> Result<Vec<String>> {
        scoped!(self, scope, false, {
            scope.search_excluded_folders().await
        })
    }
    pub(crate) async fn set_search_excluded_folders(&self, folders: &[String]) -> Result<()> {
        scoped!(self, scope, true, {
            scope.set_search_excluded_folders(folders).await
        })
    }
    /// Revalidate a whole destination page in one source snapshot. Ownership,
    /// pending receipts, membership and snoozes cannot require a query per row.
    pub(crate) async fn current_search_messages(
        &self,
        ids: &[String],
    ) -> Result<Vec<(Message, bool)>> {
        if ids.len() > 200 {
            return Err(Error::Invalid);
        }
        let ids = serde_json::to_string(ids)?;
        transaction!(self, tx, local, false, {
            let rows: Vec<(String, bool)> = if local {
                // Keep the source snapshot and every eligibility predicate.
                // Deliver one bounded batch across SQLite's worker channel.
                let batch:String=sqlx::query_scalar("SELECT COALESCE(json_group_array(json_array(m.document,EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?))),'[]') FROM messages m JOIN search_documents d ON d.message_id=m.id JOIN search_eligible_messages e ON e.id=m.id WHERE m.id IN (SELECT value FROM json_each(?)) AND d.source=json_extract(m.document,'$.provider_id') AND NOT EXISTS(SELECT 1 FROM search_dirty WHERE message_id=m.id) AND NOT EXISTS(SELECT 1 FROM search_index_outbox WHERE message_id=m.id)").bind(now()).bind(&ids).fetch_one(&mut *tx).await?;
                serde_json::from_str::<Vec<(String, i64)>>(&batch)?
                    .into_iter()
                    .map(|(raw, snoozed)| (raw, snoozed != 0))
                    .collect()
            } else {
                sqlx::query_as("SELECT m.document,EXISTS(SELECT 1 FROM snoozes s WHERE s.account_id=m.account_id AND s.thread_id=m.thread_id AND s.until>$1) FROM messages m JOIN search_documents d ON d.account_id=m.account_id AND d.message_id=m.id JOIN search_eligible_messages e ON e.account_id=m.account_id AND e.id=m.id WHERE m.account_id=$2 AND m.id IN (SELECT jsonb_array_elements_text($3::jsonb)) AND d.source=m.source AND NOT EXISTS(SELECT 1 FROM search_dirty WHERE account_id=m.account_id AND message_id=m.id) AND NOT EXISTS(SELECT 1 FROM search_index_outbox WHERE account_id=m.account_id AND message_id=m.id)").bind(now()).bind(&self.account).bind(&ids).fetch_all(&mut *tx).await?
            };
            rows.into_iter()
                .map(|(raw, snoozed)| Ok((serde_json::from_str(&raw)?, snoozed)))
                .collect()
        })
    }
    #[cfg(test)]
    pub(crate) async fn current_search_message(&self, id: &str) -> Result<Option<Message>> {
        Ok(self
            .current_search_messages(&[id.to_owned()])
            .await?
            .into_iter()
            .next()
            .map(|(message, _)| message))
    }
}

impl MailboxTransaction<'_> {
    async fn source_coverage(
        &mut self,
        expected: Option<&str>,
    ) -> Result<(crate::full_text::TextCoverage, Option<String>)> {
        let row:Option<SourceInventoryRow>=tuples!(self,"SELECT t.headers,t.bodies,t.pending,t.gaps,t.body_bytes,t.dirty,t.retrying,t.delivery_pending,t.eligible,t.processed,v.document,COALESCE(?1,p.model_revision),COALESCE(s.processed,0),CASE WHEN p.model_revision=COALESCE(?1,p.model_revision) THEN p.available ELSE -1 END,COALESCE(c.pending,0) FROM search_totals t LEFT JOIN provider_documents v ON v.key='search:database' LEFT JOIN search_semantic_profile p ON p.id=1 LEFT JOIN search_stage_totals s ON s.model_revision=COALESCE(?1,p.model_revision) AND s.lexical_revision=MAX(COALESCE(p.lexical_revision,2),?2) LEFT JOIN search_cleanup_totals c ON c.id=1 WHERE t.id=1",[expected,crate::lexical_text::REVISION],"SELECT t.headers,t.bodies,t.pending,t.gaps,t.body_bytes,t.dirty,t.retrying,t.delivery_pending,t.eligible,t.processed,v.document,COALESCE($2,p.model_revision),COALESCE(s.processed,0),CASE WHEN p.model_revision=COALESCE($2,p.model_revision) THEN p.available ELSE -1 END,COALESCE(c.pending,0) FROM search_totals t LEFT JOIN provider_documents v ON v.account_id=t.account_id AND v.key='search:database' LEFT JOIN search_semantic_profile p ON p.account_id=t.account_id LEFT JOIN search_stage_totals s ON s.account_id=t.account_id AND s.model_revision=COALESCE($2,p.model_revision) AND s.lexical_revision=GREATEST(COALESCE(p.lexical_revision,2),$3) LEFT JOIN search_cleanup_totals c ON c.account_id=t.account_id WHERE t.account_id=$1",[&self.account,expected,crate::lexical_text::REVISION],fetch_optional);
        let Some((
            headers,
            bodies,
            pending,
            gaps,
            bytes,
            dirty,
            retrying,
            delivery_pending,
            eligible,
            processed,
            identity,
            model_revision,
            semantic_processed,
            semantic_available,
            cleanup,
        )) = row
        else {
            return Ok((Default::default(), None));
        };
        Ok((
            crate::full_text::TextCoverage {
                headers,
                bodies,
                pending,
                gaps,
                bytes,
                dirty,
                retrying,
                delivery_pending,
                eligible,
                processed: if model_revision.is_some() {
                    semantic_processed
                } else {
                    processed
                },
                semantic_required: model_revision.is_some(),
                semantic_revision: model_revision.clone(),
                semantic_available: semantic_available == 1,
                semantic_failed: semantic_available == 0,
                semantic_pending: if model_revision.is_some() {
                    (eligible - semantic_processed).max(0)
                } else {
                    0
                },
                backlog: if model_revision.is_some() {
                    (eligible - semantic_processed).max(0) + cleanup
                } else {
                    dirty + pending + retrying + delivery_pending
                },
                ..Default::default()
            },
            identity,
        ))
    }
}
impl AccountRepository {
    pub(crate) async fn source_coverage(
        &self,
        expected: Option<&str>,
    ) -> Result<(crate::full_text::TextCoverage, Option<String>)> {
        scoped!(self, scope, false, {
            scope.source_coverage(expected).await
        })
    }
}

impl AccountRepository {
    pub(crate) async fn cached_search_rows(
        &self,
        filter: &CachedSearchFilter<'_>,
    ) -> Result<Vec<Message>> {
        self.cached_search(filter, None).await
    }
    pub(crate) async fn cached_search_preview_rows(
        &self,
        filter: &CachedSearchFilter<'_>,
    ) -> Result<Vec<Message>> {
        self.cached_search(filter, Some(1024)).await
    }
    async fn cached_search(
        &self,
        filter: &CachedSearchFilter<'_>,
        budget: Option<i64>,
    ) -> Result<Vec<Message>> {
        let folders = filter.folders.map(serde_json::to_string).transpose()?;
        let classification = serde_json::to_string(&filter.classification)?;
        let mut timing = crate::timing::SearchTiming::new("header_search_admission");
        transaction!(self, tx, local, false, {
            timing.stage("header_search_sql");
            let rows: Vec<String> = if local {
                let base = "SELECT m.document FROM messages m JOIN search_eligible_messages eligible ON eligible.id=m.id
             WHERE NOT EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             AND (? IS NULL OR instr(lower(json_extract(m.document,'$.subject')),lower(?))>0)
             AND (? IS NULL OR instr(lower(json_extract(m.document,'$.sender.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(m.document,'$.sender.name'),'')),lower(?))>0)
             AND (? IS NULL OR EXISTS(SELECT 1 FROM json_each(m.document,'$.to') t WHERE instr(lower(json_extract(t.value,'$.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(t.value,'$.name'),'')),lower(?))>0))
             AND (? IS NULL OR instr(lower(json_extract(m.document,'$.subject')),lower(?))>0 OR instr(lower(json_extract(m.document,'$.sender.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(m.document,'$.sender.name'),'')),lower(?))>0 OR EXISTS(SELECT 1 FROM json_each(m.document,'$.to') t WHERE instr(lower(json_extract(t.value,'$.email')),lower(?))>0 OR instr(lower(COALESCE(json_extract(t.value,'$.name'),'')),lower(?))>0))
             AND (? IS NULL OR json_extract(m.document,'$.unread')<>?)
             AND (? IS NULL OR json_extract(m.document,'$.starred')=?)
             AND (? IS NULL OR EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE f.value IN (SELECT value FROM json_each(?))))
             AND (? IS NULL OR json_extract(m.document,'$.category') IS NULL OR json_extract(m.document,'$.category')=?)
             AND (?=0 OR EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?))
             AND (?=0 OR NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?))
             AND (? IS NULL OR json_extract(m.document,'$.received_date')>=?)
             AND (? IS NULL OR json_extract(m.document,'$.received_date')<?)
             ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 50";
                let sql = if budget.is_some() {
                    base.replacen("SELECT m.document FROM messages m", "WITH boundary AS MATERIALIZED (SELECT json_extract(document,'$.received_at') AS received_at,id FROM messages ORDER BY json_extract(document,'$.received_at') DESC,id DESC LIMIT 1 OFFSET (? - 1)) SELECT m.document FROM messages m INDEXED BY messages_received_order",1)
                        .replacen("JOIN search_eligible_messages eligible", "CROSS JOIN search_eligible_messages eligible",1)
                        .replacen("WHERE NOT EXISTS", "WHERE (json_extract(m.document,'$.received_at'),m.id)>=(COALESCE((SELECT received_at FROM boundary),''),COALESCE((SELECT id FROM boundary),'')) AND NOT EXISTS",1)
                } else {
                    base.to_owned()
                };
                let sql=sql.replace("ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 50", &format!("{} ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 50",crate::classification::CLASSIFICATION_SQL)).replace("ON eligible.id=m.id", "ON eligible.id=m.id CROSS JOIN (SELECT ? AS filter) cf");
                let mut request = sqlx::query_scalar(&sql);
                if let Some(budget) = budget {
                    request = request.bind(budget);
                }
                request
                    .bind(&classification)
                    .bind(filter.subject)
                    .bind(filter.subject)
                    .bind(filter.from)
                    .bind(filter.from)
                    .bind(filter.from)
                    .bind(filter.to)
                    .bind(filter.to)
                    .bind(filter.to)
                    .bind(filter.text)
                    .bind(filter.text)
                    .bind(filter.text)
                    .bind(filter.text)
                    .bind(filter.text)
                    .bind(filter.text)
                    .bind(filter.seen)
                    .bind(filter.seen)
                    .bind(filter.starred)
                    .bind(filter.starred)
                    .bind(&folders)
                    .bind(&folders)
                    .bind(filter.category)
                    .bind(filter.category)
                    .bind(filter.snoozed_only)
                    .bind(now())
                    .bind(filter.hide_snoozed)
                    .bind(now())
                    .bind(filter.since)
                    .bind(filter.since)
                    .bind(filter.before)
                    .bind(filter.before)
                    .fetch_all(&mut *tx)
                    .await?
            } else {
                let base = "SELECT m.document FROM messages m JOIN search_eligible_messages e ON e.account_id=m.account_id AND e.id=m.id
WHERE m.account_id=$1
AND ($3::TEXT IS NULL OR mail_search_contains(m.document::jsonb->>'subject',$3))
AND ($4::TEXT IS NULL OR mail_search_contains(m.document::jsonb->'sender'->>'email',$4) OR mail_search_contains(COALESCE(m.document::jsonb->'sender'->>'name',''),$4))
AND ($5::TEXT IS NULL OR EXISTS(SELECT 1 FROM jsonb_array_elements(m.document::jsonb->'to') t WHERE mail_search_contains(t->>'email',$5) OR mail_search_contains(COALESCE(t->>'name',''),$5)))
AND ($6::TEXT IS NULL OR mail_search_contains(m.document::jsonb->>'subject',$6) OR mail_search_contains(m.document::jsonb->'sender'->>'email',$6) OR mail_search_contains(COALESCE(m.document::jsonb->'sender'->>'name',''),$6) OR EXISTS(SELECT 1 FROM jsonb_array_elements(m.document::jsonb->'to') t WHERE mail_search_contains(t->>'email',$6) OR mail_search_contains(COALESCE(t->>'name',''),$6)))
AND ($7::BOOLEAN IS NULL OR m.unread<>$7) AND ($8::BOOLEAN IS NULL OR m.starred=$8)
AND ($9::TEXT IS NULL OR EXISTS(SELECT 1 FROM unnest(m.folder_ids) f WHERE f IN (SELECT jsonb_array_elements_text($9::jsonb))))
AND ($10::TEXT IS NULL OR m.category IS NULL OR m.category=$10)
AND (NOT $11 OR EXISTS(SELECT 1 FROM snoozes s WHERE s.account_id=m.account_id AND s.thread_id=m.thread_id AND s.until>$2))
AND (NOT $12 OR NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.account_id=m.account_id AND s.thread_id=m.thread_id AND s.until>$2))
AND ($13::TEXT IS NULL OR m.document::jsonb->>'received_date'>=$13)
AND ($14::TEXT IS NULL OR m.document::jsonb->>'received_date'<$14)
ORDER BY m.received_at DESC,m.id COLLATE \"C\" DESC LIMIT 50";
                let sql = if budget.is_some() {
                    base.replacen("SELECT m.document FROM messages m", "WITH boundary AS MATERIALIZED (SELECT received_at,id FROM messages WHERE account_id=$1 ORDER BY received_at DESC,id COLLATE \"C\" DESC LIMIT 1 OFFSET ($16::BIGINT - 1)) SELECT m.document FROM messages m",1)
                        .replacen("WHERE m.account_id=$1", "WHERE m.account_id=$1 AND (m.received_at,m.id COLLATE \"C\")>=(COALESCE((SELECT received_at FROM boundary),''),COALESCE((SELECT id FROM boundary),''))",1)
                } else {
                    base.to_owned()
                };
                let sql=sql.replace("ORDER BY m.received_at DESC,m.id COLLATE \"C\" DESC LIMIT 50", &format!("{} ORDER BY m.received_at DESC,m.id COLLATE \"C\" DESC LIMIT 50",crate::classification::CLASSIFICATION_PG_SQL)).replace("ON e.account_id=m.account_id AND e.id=m.id", "ON e.account_id=m.account_id AND e.id=m.id CROSS JOIN (SELECT $15::TEXT AS filter) cf");
                let mut request = sqlx::query_scalar(&sql)
                    .bind(&self.account)
                    .bind(now())
                    .bind(filter.subject)
                    .bind(filter.from)
                    .bind(filter.to)
                    .bind(filter.text)
                    .bind(filter.seen)
                    .bind(filter.starred)
                    .bind(&folders)
                    .bind(filter.category)
                    .bind(filter.snoozed_only)
                    .bind(filter.hide_snoozed)
                    .bind(filter.since)
                    .bind(filter.before)
                    .bind(&classification);
                if let Some(budget) = budget {
                    request = request.bind(budget);
                }
                request.fetch_all(&mut *tx).await?
            };
            timing.stage("header_search_decode");
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
}
