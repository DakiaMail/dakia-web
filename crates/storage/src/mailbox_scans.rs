use super::*;
type InboxScanCoverage = (Option<i64>, bool, Option<String>, Option<i64>);
struct ArrivalPolicy<'a> {
    owner: &'a str,
    subscriptions: &'a [serde_json::Value],
    rules: &'a [MailRule],
}

impl MailboxTransaction<'_> {
    async fn seed_memberships(&mut self, folder: &str) -> Result<u64> {
        Ok(write!(self,"INSERT INTO folder_memberships(folder,message_id,generation) SELECT ?,m.id,-1 FROM messages m WHERE EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') WHERE value=?) ON CONFLICT DO NOTHING",[folder,folder],"INSERT INTO folder_memberships(account_id,folder,message_id,generation) SELECT $1,$2,m.id,-1 FROM messages m WHERE m.account_id=$1 AND m.folder_ids @> ARRAY[$2]::TEXT[] ON CONFLICT DO NOTHING",[&self.account,folder]))
    }
    async fn scan_state(&mut self, folder: &str, at: i64) -> Result<Option<(i64, Option<String>)>> {
        write!(
            self,
            "INSERT INTO folder_scans(folder) VALUES(?) ON CONFLICT DO NOTHING",
            [folder],
            "INSERT INTO folder_scans(account_id,folder) VALUES($1,$2) ON CONFLICT DO NOTHING",
            [&self.account, folder]
        );
        write!(self,"UPDATE folder_scans SET generation=generation+1,completed_at=NULL,coverage_limited=0 WHERE folder=? AND continuation IS NULL AND (completed_at IS NOT NULL OR coverage_limited=1) AND COALESCE(last_scanned_at,0)<=?",[folder,at-300],"UPDATE folder_scans SET generation=generation+1,completed_at=NULL,coverage_limited=FALSE WHERE account_id=$1 AND folder=$2 AND continuation IS NULL AND (completed_at IS NOT NULL OR coverage_limited) AND COALESCE(last_scanned_at,0)<=$3",[&self.account,folder,at-300]);
        let (generation,continuation,completed,limited):(i64,Option<String>,Option<i64>,bool)=tuples!(self,"SELECT generation,continuation,completed_at,coverage_limited FROM folder_scans WHERE folder=?",[folder],"SELECT generation,continuation,completed_at,coverage_limited FROM folder_scans WHERE account_id=$1 AND folder=$2",[&self.account,folder],fetch_one);
        if completed.is_some() || (limited && continuation.is_none()) {
            return Ok(None);
        }
        if continuation.is_none() {
            self.seed_memberships(folder).await?;
        }
        Ok(Some((generation, continuation)))
    }
    async fn reset_scan(&mut self, folder: &str) -> Result<()> {
        write!(self,"UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=0 WHERE folder=?",[folder],"UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=FALSE WHERE account_id=$1 AND folder=$2",[&self.account,folder]);
        Ok(())
    }
    async fn clear_verification(&mut self, folder: &str) -> Result<()> {
        let modseq = format!("sync:modseq:{folder}");
        let verified = format!("sync:verified:{folder}");
        write!(
            self,
            "DELETE FROM provider_documents WHERE key IN (?,?)",
            [&modseq, &verified],
            "DELETE FROM provider_documents WHERE account_id=$1 AND key IN ($2,$3)",
            [&self.account, &modseq, &verified]
        );
        Ok(())
    }
    async fn completed_scan_due(&mut self, folder: &str, at: i64) -> Result<(Option<i64>, u64)> {
        let generation:Option<i64>=value!(self,"SELECT generation FROM folder_scans WHERE folder=? AND continuation IS NULL AND completed_at IS NOT NULL AND coverage_limited=0 AND COALESCE(last_scanned_at,0)<=?",[folder,at-300],"SELECT generation FROM folder_scans WHERE account_id=$1 AND folder=$2 AND continuation IS NULL AND completed_at IS NOT NULL AND NOT coverage_limited AND COALESCE(last_scanned_at,0)<=$3",[&self.account,folder,at-300],fetch_optional);
        let seeded = if generation.is_some() {
            self.seed_memberships(folder).await?
        } else {
            0
        };
        if seeded > 0 {
            self.reset_scan(folder).await?;
            self.clear_verification(folder).await?;
            return Ok((None, seeded));
        }
        Ok((generation, 0))
    }
    async fn confirm_unchanged_scan(
        &mut self,
        folder: &str,
        generation: i64,
        at: i64,
    ) -> Result<()> {
        let changed=write!(self,"UPDATE folder_scans SET last_scanned_at=? WHERE folder=? AND generation=? AND continuation IS NULL AND completed_at IS NOT NULL AND coverage_limited=0",[at,folder,generation],"UPDATE folder_scans SET last_scanned_at=$1 WHERE account_id=$2 AND folder=$3 AND generation=$4 AND continuation IS NULL AND completed_at IS NOT NULL AND NOT coverage_limited",[at,&self.account,folder,generation]);
        if changed != 1 {
            return Err(Error::Conflict);
        }
        let key = format!("sync:verified:{folder}");
        write!(self,"INSERT INTO provider_documents(key,document) SELECT ?,json_object('generation',generation,'uidvalidity',uidvalidity,'verified_at',last_scanned_at) FROM folder_scans WHERE folder=? ON CONFLICT(key) DO UPDATE SET document=excluded.document",[&key,folder],"INSERT INTO provider_documents(account_id,key,document) SELECT $1,$2,jsonb_build_object('generation',generation,'uidvalidity',uidvalidity,'verified_at',last_scanned_at)::TEXT FROM folder_scans WHERE account_id=$1 AND folder=$3 ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&self.account,&key,folder]);
        Ok(())
    }
    async fn observe_memberships(
        &mut self,
        owner: &str,
        folder: &str,
        generation: i64,
        rows: &[Message],
        subscriptions: &[serde_json::Value],
        rules: &[MailRule],
    ) -> Result<()> {
        let boundary: Option<i64> = value!(
            self,
            "SELECT initial_boundary FROM folder_scans WHERE folder=?",
            [folder],
            "SELECT initial_boundary FROM folder_scans WHERE account_id=$1 AND folder=$2",
            [&self.account, folder],
            fetch_one
        );
        for message in rows {
            if message.account_id != self.account {
                return Err(Error::Invalid);
            }
            let known:bool=value!(self,"SELECT EXISTS(SELECT 1 FROM folder_memberships WHERE folder=? AND message_id=?)",[folder,&message.id],"SELECT EXISTS(SELECT 1 FROM folder_memberships WHERE account_id=$1 AND folder=$2 AND message_id=$3)",[&self.account,folder,&message.id],fetch_one);
            if !known
                && folder.eq_ignore_ascii_case("INBOX")
                && !rules.iter().any(|rule| {
                    rule.enabled && {
                        let RuleCondition::SenderIs { email } = &rule.condition;
                        message.sender.email.trim().to_lowercase() == *email
                    }
                })
                && boundary.is_some_and(|boundary| {
                    DateTime::parse_from_rfc3339(&message.received_at)
                        .is_ok_and(|at| at.timestamp() > boundary)
                })
            {
                let inserted=write!(self,"INSERT INTO notification_dedup(message_id) VALUES(?) ON CONFLICT DO NOTHING",[&message.id],"INSERT INTO notification_dedup(account_id,message_id) VALUES($1,$2) ON CONFLICT DO NOTHING",[&self.account,&message.id]);
                if inserted == 1 {
                    write!(self,"DELETE FROM snoozes WHERE thread_id=? AND arrival_boundary<?",[&message.thread_id,&message.received_at],"DELETE FROM snoozes WHERE account_id=$1 AND thread_id=$2 AND arrival_boundary<$3",[&self.account,&message.thread_id,&message.received_at]);
                    for subscription in subscriptions {
                        let id = subscription["id"].as_str().ok_or(Error::Invalid)?;
                        let job = serde_json::json!({"id":format!("{}:{}:{id}",self.account,message.id),"account_id":self.account,"user_id":owner,"message_id":message.id,"subscription_id":id,"scope":{"account_id":self.account,"user_id":owner},"subscription":subscription,"deadline":(Utc::now()+Duration::minutes(5)).to_rfc3339(),"initial_sync":false});
                        let raw = serde_json::to_string(&job)?;
                        write!(self,"INSERT INTO push_jobs(message_id,subscription_id,document,state) VALUES(?,?,?,'pending') ON CONFLICT DO NOTHING",[&message.id,id,&raw],"INSERT INTO push_jobs(account_id,message_id,subscription_id,document,state) VALUES($1,$2,$3,$4,'pending') ON CONFLICT DO NOTHING",[&self.account,&message.id,id,&raw]);
                    }
                }
            }
            write!(self,"INSERT INTO folder_memberships(folder,message_id,generation) VALUES(?,?,?) ON CONFLICT(folder,message_id) DO UPDATE SET generation=excluded.generation",[folder,&message.id,generation],"INSERT INTO folder_memberships(account_id,folder,message_id,generation) VALUES($1,$2,$3,$4) ON CONFLICT(account_id,folder,message_id) DO UPDATE SET generation=excluded.generation",[&self.account,folder,&message.id,generation]);
        }
        Ok(())
    }
    async fn finish_snapshot(&mut self, folder: &str, generation: i64, at: i64) -> Result<()> {
        write!(self,"UPDATE folder_scans SET initial_boundary=COALESCE(initial_boundary,?) WHERE folder=?",[at,folder],"UPDATE folder_scans SET initial_boundary=COALESCE(initial_boundary,$1) WHERE account_id=$2 AND folder=$3",[at,&self.account,folder]);
        let stale:Vec<String>=value!(self,"SELECT messages.document FROM messages JOIN folder_memberships ON messages.id=folder_memberships.message_id WHERE folder_memberships.folder=? AND folder_memberships.generation<>? AND NOT EXISTS(SELECT 1 FROM operations o WHERE json_extract(o.document,'$.target_id')=messages.id AND json_extract(o.document,'$.status')='pending' AND json_extract(o.document,'$.change.kind') IN ('archive','spam'))",[folder,generation],"SELECT m.document FROM messages m JOIN folder_memberships f ON f.account_id=m.account_id AND f.message_id=m.id WHERE m.account_id=$1 AND f.folder=$2 AND f.generation<>$3 AND NOT EXISTS(SELECT 1 FROM operations o WHERE o.account_id=$1 AND o.target_id=m.id AND o.status='pending' AND o.change_kind IN ('archive','spam'))",[&self.account,folder,generation],fetch_all);
        for raw in stale {
            let mut message: Message = serde_json::from_str(&raw)?;
            message.folder_ids.retain(|f| {
                f != folder && !(folder.eq_ignore_ascii_case("INBOX") && f == "\\Inbox")
            });
            message.revision = message.revision.checked_add(1).ok_or(Error::Invalid)?;
            self.store_message(&message).await?;
            self.event(message.revision, "message_changed", &message.id)
                .await?;
        }
        write!(self,"DELETE FROM folder_memberships WHERE folder=? AND generation<>? AND NOT EXISTS(SELECT 1 FROM operations o WHERE json_extract(o.document,'$.target_id')=folder_memberships.message_id AND json_extract(o.document,'$.status')='pending' AND json_extract(o.document,'$.change.kind') IN ('archive','spam'))",[folder,generation],"DELETE FROM folder_memberships f WHERE f.account_id=$1 AND f.folder=$2 AND f.generation<>$3 AND NOT EXISTS(SELECT 1 FROM operations o WHERE o.account_id=$1 AND o.target_id=f.message_id AND o.status='pending' AND o.change_kind IN ('archive','spam'))",[&self.account,folder,generation]);
        Ok(())
    }
    async fn record_scan(
        &mut self,
        policy: ArrivalPolicy<'_>,
        folder: &str,
        generation: i64,
        page: &SearchPage,
        at: i64,
    ) -> Result<()> {
        let previous_limited: bool = value!(
            self,
            "SELECT coverage_limited FROM folder_scans WHERE folder=?",
            [folder],
            "SELECT coverage_limited FROM folder_scans WHERE account_id=$1 AND folder=$2",
            [&self.account, folder],
            fetch_one
        );
        let limited = previous_limited
            || page.coverage.is_empty()
            || page
                .coverage
                .iter()
                .any(|c| c.state != CoverageState::Complete);
        let complete = page.continuation.is_none() && !limited;
        let changed=write!(self,"UPDATE folder_scans SET continuation=?,completed_at=?,last_scanned_at=COALESCE(?,last_scanned_at),coverage_limited=?,uidvalidity=(SELECT CAST(document AS INTEGER) FROM provider_documents WHERE key='thread:epoch:'||folder_scans.folder) WHERE folder=? AND generation=? AND completed_at IS NULL AND NOT (coverage_limited=1 AND continuation IS NULL)",[&page.continuation,complete.then_some(at),page.continuation.is_none().then_some(at),limited,folder,generation],"UPDATE folder_scans f SET continuation=$1,completed_at=$2,last_scanned_at=COALESCE($3,last_scanned_at),coverage_limited=$4,uidvalidity=(SELECT document::BIGINT FROM provider_documents WHERE account_id=$5 AND key='thread:epoch:'||f.folder) WHERE f.account_id=$5 AND f.folder=$6 AND f.generation=$7 AND f.completed_at IS NULL AND NOT(f.coverage_limited AND f.continuation IS NULL)",[&page.continuation,complete.then_some(at),page.continuation.is_none().then_some(at),limited,&self.account,folder,generation]);
        if changed != 1 {
            return Err(Error::Conflict);
        }
        self.observe_memberships(
            policy.owner,
            folder,
            generation,
            &page.rows,
            policy.subscriptions,
            policy.rules,
        )
        .await?;
        if complete {
            self.finish_snapshot(folder, generation, at).await?;
        }
        Ok(())
    }
    async fn record_incremental(
        &mut self,
        policy: ArrivalPolicy<'_>,
        folder: &str,
        rows: &[Message],
        checkpoint: &str,
        at: i64,
    ) -> Result<()> {
        write!(
            self,
            "INSERT INTO folder_scans(folder) VALUES(?) ON CONFLICT DO NOTHING",
            [folder],
            "INSERT INTO folder_scans(account_id,folder) VALUES($1,$2) ON CONFLICT DO NOTHING",
            [&self.account, folder]
        );
        let generation: i64 = value!(
            self,
            "SELECT generation FROM folder_scans WHERE folder=?",
            [folder],
            "SELECT generation FROM folder_scans WHERE account_id=$1 AND folder=$2",
            [&self.account, folder],
            fetch_one
        );
        self.observe_memberships(
            policy.owner,
            folder,
            generation,
            rows,
            policy.subscriptions,
            policy.rules,
        )
        .await?;
        write!(self,"UPDATE folder_scans SET initial_boundary=COALESCE(initial_boundary,?) WHERE folder=?",[at,folder],"UPDATE folder_scans SET initial_boundary=COALESCE(initial_boundary,$1) WHERE account_id=$2 AND folder=$3",[at,&self.account,folder]);
        let key = format!("sync:head:{folder}");
        write!(self,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document",[&key,checkpoint],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&self.account,&key,checkpoint]);
        Ok(())
    }
}

impl AccountRepository {
    pub(crate) async fn inbox_rows(
        &self,
        before: Option<(&str, &str)>,
        category: Option<&str>,
        filter: &ClassificationFilter,
    ) -> Result<Vec<Message>> {
        let rows: Vec<String> = match &self.backend {
            Backend::Sqlite(db) => {
                let mut tx = db.begin().await?;
                // The static candidate predicate lets SQLite use the sparse attention index.
                // Expiry and manual dismissal are still checked by CLASSIFICATION_SQL.
                let attention = if filter.attention {
                    "AND (json_extract(m.document,'$.classification.attention_override')=1 OR json_array_length(m.document,'$.classification.attention')>0)"
                } else {
                    ""
                };
                let source = if filter.email_type.is_some() {
                    "message_types t CROSS JOIN messages m ON m.id=t.message_id"
                } else if filter.transactions {
                    "(SELECT DISTINCT message_id FROM message_types WHERE email_type IN ('receipt','invoice')) tx CROSS JOIN messages m ON m.id=tx.message_id"
                } else {
                    "messages m"
                };
                let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
                    "SELECT m.document FROM {source} CROSS JOIN (SELECT "
                ));
                q.push_bind(serde_json::to_string(filter)?)
                    .push(" AS filter) cf WHERE 1=1");
                if let Some(kind) = &filter.inbox_kind {
                    q.push(" AND COALESCE(json_extract(m.document,'$.classification.kind'),'personal')=")
                .push_bind(match kind {
                    InboxKind::Personal => "personal",
                    InboxKind::Automated => "automated",
                });
                }
                if let Some(email_type) = &filter.email_type {
                    let value = serde_json::to_value(email_type)?;
                    q.push(" AND t.email_type=")
                        .push_bind(value.as_str().ok_or(Error::Invalid)?.to_owned());
                }
                q.push(format!(
            r#" AND instr(upper(json_extract(m.document,'$.folder_ids')),'"INBOX"')>0
             AND EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE upper(f.value)='INBOX')
             AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END
             AND NOT EXISTS(SELECT 1 FROM operations o WHERE json_extract(o.document,'$.target_id')=m.id AND json_extract(o.document,'$.status')='pending' AND json_extract(o.document,'$.change.kind') IN ('archive','spam','trash'))
             AND NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id') AND s.until>?)
             {attention}
             {classification}
             AND (? IS NULL OR json_extract(m.document,'$.category') IS NULL OR json_extract(m.document,'$.category')=?)
             AND (? IS NULL OR (json_extract(m.document,'$.received_at'),m.id)<(?,?))
             "#, classification=crate::classification::CLASSIFICATION_SQL,
        ));
                q.push(if filter.email_type.is_some() {
                    " ORDER BY t.received_at DESC,t.message_id DESC LIMIT 51"
                } else {
                    " ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 51"
                });
                // Bind nullable predicates in the static tail without widening the page.
                let mut query = q.build_query_scalar::<String>();
                query = query
                    .bind(now())
                    .bind(category)
                    .bind(category)
                    .bind(before.map(|b| b.0))
                    .bind(before.map(|b| b.0))
                    .bind(before.map(|b| b.1));
                query.fetch_all(&mut *tx).await?
            }
            Backend::Postgres(db) => {
                let mut tx = self.begin_postgres(db, false).await?;
                let source = if filter.email_type.is_some() {
                    "message_types t JOIN messages m ON m.account_id=t.account_id AND m.id=t.message_id"
                } else if filter.transactions {
                    "(SELECT DISTINCT account_id,message_id FROM message_types WHERE email_type IN ('receipt','invoice')) t JOIN messages m ON m.account_id=t.account_id AND m.id=t.message_id"
                } else {
                    "messages m"
                };
                let mut q = sqlx::QueryBuilder::<sqlx::Postgres>::new(format!(
                    "SELECT m.document FROM {source} WHERE m.account_id="
                ));
                q.push_bind(&self.account).push(" AND m.upper_folders @> ARRAY['INBOX']::TEXT[] AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=m.account_id AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch) AND NOT EXISTS(SELECT 1 FROM operations o WHERE o.account_id=m.account_id AND o.target_id=m.id AND o.status='pending' AND o.change_kind IN ('archive','spam','trash')) AND NOT EXISTS(SELECT 1 FROM snoozes s WHERE s.account_id=m.account_id AND s.thread_id=m.thread_id AND s.until>").push_bind(now()).push(")");
                if let Some(kind) = &filter.inbox_kind {
                    q.push(
                        " AND COALESCE(m.document::jsonb#>>'{classification,kind}','personal')=",
                    )
                    .push_bind(crate::classification::wire(kind)?);
                }
                if let Some(email_type) = &filter.email_type {
                    q.push(" AND t.email_type=")
                        .push_bind(crate::classification::wire(email_type)?);
                }
                if filter.attention {
                    q.push(" AND ((m.document::jsonb#>>'{classification,attention_override}')::BOOLEAN IS TRUE OR jsonb_array_length(m.document::jsonb#>'{classification,attention}')>0)");
                }
                crate::classification::predicate!(q, false, "m.document", filter);
                if let Some(category) = category {
                    q.push(" AND (m.category IS NULL OR m.category=")
                        .push_bind(category)
                        .push(")");
                }
                if let Some((date, id)) = before {
                    q.push(" AND (m.received_at,m.id COLLATE \"C\")<(")
                        .push_bind(date)
                        .push(",")
                        .push_bind(id)
                        .push(")");
                }
                q.push(if filter.email_type.is_some() {
                    " ORDER BY t.received_at DESC,t.message_id COLLATE \"C\" DESC LIMIT 51"
                } else {
                    " ORDER BY m.received_at DESC,m.id COLLATE \"C\" DESC LIMIT 51"
                });
                q.build_query_scalar().fetch_all(&mut *tx).await?
            }
        };
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }
    pub(crate) async fn inbox_coverage(&self) -> Result<AccountCoverage> {
        transaction!(self, tx, local, false, {
            let row: Option<InboxScanCoverage> = if local {
                sqlx::query_as("SELECT completed_at,coverage_limited,continuation,last_scanned_at FROM folder_scans WHERE folder='INBOX'").fetch_optional(&mut *tx).await?
            } else {
                sqlx::query_as("SELECT completed_at,coverage_limited,continuation,last_scanned_at FROM folder_scans WHERE account_id=$1 AND folder='INBOX'").bind(&self.account).fetch_optional(&mut *tx).await?
            };
            let (completed, limited, continuation, last_scanned_at) =
                row.unwrap_or((None, false, None, None));
            let error:Option<String>=scalar!(local,tx,"SELECT document FROM provider_documents WHERE key='inbox:refresh-error'",[],"SELECT document FROM provider_documents WHERE account_id=$1 AND key='inbox:refresh-error'",[&self.account],fetch_optional);
            Ok(AccountCoverage {
                account_id: self.account.clone(),
                folder_ids: vec!["INBOX".into()],
                state: if error.is_some() {
                    CoverageState::Error
                } else if completed.is_some_and(|at| Utc::now().timestamp() - at < 300) {
                    CoverageState::Complete
                } else if limited && continuation.is_none() {
                    CoverageState::Partial
                } else {
                    CoverageState::Pending
                },
                error,
                inbox: Some(InboxFreshness {
                    last_scanned_at,
                    refreshing: completed.is_none() && !(limited && continuation.is_none()),
                }),
            })
        })
    }
    pub(crate) async fn seed_inbox_memberships(&self) -> Result<()> {
        transaction!(self, tx, local, true, {
            execute!(local,tx,"INSERT INTO folder_memberships(folder,message_id,generation) SELECT 'INBOX',m.id,-1 FROM messages m WHERE CASE WHEN EXISTS(SELECT 1 FROM folder_memberships known WHERE known.folder='INBOX' AND known.message_id=m.id) THEN 0 ELSE EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f WHERE upper(f.value)='INBOX') END ON CONFLICT DO NOTHING",[],"INSERT INTO folder_memberships(account_id,folder,message_id,generation) SELECT $1,'INBOX',m.id,-1 FROM messages m WHERE m.account_id=$1 AND m.upper_folders @> ARRAY['INBOX']::TEXT[] ON CONFLICT DO NOTHING",[&self.account]);
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn clear_provider_document(&self, key: &str) -> Result<()> {
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "DELETE FROM provider_documents WHERE key=?",
                [key],
                "DELETE FROM provider_documents WHERE account_id=$1 AND key=$2",
                [&self.account, key]
            );
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn scan_state(&self, folder: &str) -> Result<Option<(i64, Option<String>)>> {
        scoped!(self, scope, true, {
            scope.scan_state(folder, Utc::now().timestamp()).await
        })
    }
    pub(crate) async fn reset_scan(&self, folder: &str) -> Result<()> {
        scoped!(self, scope, true, { scope.reset_scan(folder).await })
    }
    pub(crate) async fn completed_scan_due(&self, folder: &str) -> Result<Option<i64>> {
        let (generation, seeded) = scoped!(self, scope, true, {
            scope
                .completed_scan_due(folder, Utc::now().timestamp())
                .await
        })?;
        if seeded > 0 {
            tracing::info!(target:"dakia_server::diagnostics",operation="snapshot_membership_repair",rows=seeded,outcome="scheduled","mail_work");
        }
        Ok(generation)
    }
    pub(crate) async fn confirm_unchanged_scan(&self, folder: &str, generation: i64) -> Result<()> {
        scoped!(self, scope, true, {
            scope
                .confirm_unchanged_scan(folder, generation, Utc::now().timestamp())
                .await
        })
    }
    pub(crate) async fn record_scan(
        &self,
        folder: &str,
        generation: i64,
        page: &SearchPage,
        subscriptions: &[serde_json::Value],
        rules: &[MailRule],
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope
                .record_scan(
                    ArrivalPolicy {
                        owner: &self.owner,
                        subscriptions,
                        rules,
                    },
                    folder,
                    generation,
                    page,
                    Utc::now().timestamp(),
                )
                .await
        })
    }
    pub(crate) async fn record_incremental(
        &self,
        folder: &str,
        rows: &[Message],
        checkpoint: &str,
        subscriptions: &[serde_json::Value],
        rules: &[MailRule],
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope
                .record_incremental(
                    ArrivalPolicy {
                        owner: &self.owner,
                        subscriptions,
                        rules,
                    },
                    folder,
                    rows,
                    checkpoint,
                    Utc::now().timestamp(),
                )
                .await
        })
    }
    pub(crate) async fn scan_generation(&self, folder: &str) -> Result<Option<i64>> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(
                local,
                tx,
                "SELECT generation FROM folder_scans WHERE folder=?",
                [folder],
                "SELECT generation FROM folder_scans WHERE account_id=$1 AND folder=$2",
                [&self.account, folder],
                fetch_optional
            ))
        })
    }
    pub(crate) async fn terminal_scan_folders(&self) -> Result<Vec<String>> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT f.folder FROM folder_scans f LEFT JOIN provider_documents e ON e.key='thread:epoch:'||f.folder WHERE f.continuation IS NULL AND (f.completed_at IS NOT NULL OR f.coverage_limited=1) AND f.uidvalidity IS CAST(e.document AS INTEGER)",[],"SELECT f.folder FROM folder_scans f LEFT JOIN provider_documents e ON e.account_id=f.account_id AND e.key='thread:epoch:'||f.folder WHERE f.account_id=$1 AND f.continuation IS NULL AND (f.completed_at IS NOT NULL OR f.coverage_limited) AND f.uidvalidity IS NOT DISTINCT FROM e.document::BIGINT",[&self.account],fetch_all))
        })
    }
    pub(crate) async fn verified_snapshot_current(&self, folder: &str) -> Result<bool> {
        let at = Utc::now().timestamp();
        let verified = format!("sync:verified:{folder}");
        let epoch = format!("thread:epoch:{folder}");
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM folder_scans f JOIN provider_documents v ON v.key=? JOIN provider_documents e ON e.key=? WHERE f.folder=? AND f.completed_at IS NOT NULL AND f.continuation IS NULL AND f.coverage_limited=0 AND f.generation=json_extract(v.document,'$.generation') AND f.uidvalidity=json_extract(v.document,'$.uidvalidity') AND f.uidvalidity=CAST(e.document AS INTEGER) AND json_extract(v.document,'$.verified_at')>? AND json_extract(v.document,'$.verified_at')<=?)",[&verified,&epoch,folder,at-300,at],"SELECT EXISTS(SELECT 1 FROM folder_scans f JOIN provider_documents v ON v.account_id=f.account_id AND v.key=$2 JOIN provider_documents e ON e.account_id=f.account_id AND e.key=$3 WHERE f.account_id=$1 AND f.folder=$4 AND f.completed_at IS NOT NULL AND f.continuation IS NULL AND NOT f.coverage_limited AND f.generation=(v.document::jsonb->>'generation')::BIGINT AND f.uidvalidity=(v.document::jsonb->>'uidvalidity')::BIGINT AND f.uidvalidity=e.document::BIGINT AND (v.document::jsonb->>'verified_at')::BIGINT>$5 AND (v.document::jsonb->>'verified_at')::BIGINT<=$6)",[&self.account,&verified,&epoch,folder,at-300,at],fetch_one))
        })
    }
    pub(crate) async fn invalidate_stale_reference(
        &self,
        message: &str,
        folder: &str,
        reference: &str,
    ) -> Result<bool> {
        let key = format!("reference:{message}");
        scoped!(self, scope, true, {
            let changed=write!(scope,"UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=0 WHERE folder=? AND completed_at IS NOT NULL AND EXISTS(SELECT 1 FROM provider_documents WHERE key=? AND document=?) AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=? AND EXISTS(SELECT 1 FROM json_each(document,'$.folder_ids') WHERE value=?))",[folder,&key,reference,message,reference,folder],"UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=FALSE WHERE account_id=$1 AND folder=$2 AND completed_at IS NOT NULL AND EXISTS(SELECT 1 FROM provider_documents WHERE account_id=$1 AND key=$3 AND document=$4) AND EXISTS(SELECT 1 FROM messages WHERE account_id=$1 AND id=$5 AND source=$4 AND folder_ids @> ARRAY[$2]::TEXT[])",[&self.account,folder,&key,reference,message])==1;
            if changed {
                scope.clear_verification(folder).await?;
            }
            Ok::<_, Error>(changed)
        })
    }
}

impl AccountRepository {
    pub(crate) async fn begin_graph_scan(&self, folder: &str) -> Result<()> {
        scoped!(self, scope, true, {
            let at = Utc::now().timestamp();
            write!(scope,"INSERT INTO folder_scans(folder,initial_boundary) VALUES(?,?) ON CONFLICT DO NOTHING",[folder,at],"INSERT INTO folder_scans(account_id,folder,initial_boundary) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",[&scope.account,folder,at]);
            scope.reset_scan(folder).await?;
            scope.seed_memberships(folder).await?;
            let key = format!("graph:cursor:{folder}");
            write!(
                scope,
                "DELETE FROM provider_documents WHERE key=?",
                [&key],
                "DELETE FROM provider_documents WHERE account_id=$1 AND key=$2",
                [&scope.account, &key]
            );
            Ok::<_, Error>(())
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn commit_graph_page(
        &self,
        folder: &str,
        rows: &[Message],
        removed: &[String],
        cursor: &str,
        initial: bool,
        complete: bool,
        subscriptions: &[serde_json::Value],
        rules: &[MailRule],
    ) -> Result<()> {
        scoped!(self, scope, true, {
            let at = Utc::now().timestamp();
            write!(scope,"INSERT INTO folder_scans(folder,initial_boundary) VALUES(?,?) ON CONFLICT DO NOTHING",[folder,at],"INSERT INTO folder_scans(account_id,folder,initial_boundary) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",[&scope.account,folder,at]);
            let generation: i64 = value!(
                scope,
                "SELECT generation FROM folder_scans WHERE folder=?",
                [folder],
                "SELECT generation FROM folder_scans WHERE account_id=$1 AND folder=$2",
                [&scope.account, folder],
                fetch_one
            );
            scope
                .observe_memberships(&self.owner, folder, generation, rows, subscriptions, rules)
                .await?;
            for provider_id in removed {
                let identity = format!("graph:{provider_id}");
                let observation:Option<(String,String)>=tuples!(scope,"SELECT m.document,r.document FROM messages m JOIN message_identities i ON i.message_id=m.id JOIN provider_documents r ON r.key='reference:'||m.id WHERE i.provider_id=?",[&identity],"SELECT m.document,r.document FROM messages m JOIN message_identities i ON i.account_id=m.account_id AND i.message_id=m.id JOIN provider_documents r ON r.account_id=m.account_id AND r.key='reference:'||m.id WHERE m.account_id=$1 AND i.provider_id=$2",[&scope.account,&identity],fetch_optional);
                let Some((raw, reference)) = observation else {
                    continue;
                };
                let mut message: Message = serde_json::from_str(&raw)?;
                let reference: serde_json::Value = serde_json::from_str(&reference)?;
                if reference["graph_id"].as_str() != Some(provider_id)
                    && reference["folder"].as_str() == Some(folder)
                {
                    continue;
                }
                if message.folder_ids.iter().any(|f| f == folder) {
                    message.folder_ids.retain(|f| f != folder);
                    scope.upsert_message(message.clone()).await?;
                }
                write!(scope,"DELETE FROM folder_memberships WHERE folder=? AND message_id=?",[folder,&message.id],"DELETE FROM folder_memberships WHERE account_id=$1 AND folder=$2 AND message_id=$3",[&scope.account,folder,&message.id]);
            }
            if initial && complete {
                scope.finish_snapshot(folder, generation, at).await?;
            }
            if complete {
                write!(scope,"UPDATE folder_scans SET continuation=NULL,completed_at=?,last_scanned_at=?,coverage_limited=0,uidvalidity=1 WHERE folder=?",[at,at,folder],"UPDATE folder_scans SET continuation=NULL,completed_at=$1,last_scanned_at=$1,coverage_limited=FALSE,uidvalidity=1 WHERE account_id=$2 AND folder=$3",[at,&scope.account,folder]);
            }
            let key = format!("graph:cursor:{folder}");
            write!(scope,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document",[&key,cursor],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&scope.account,&key,cursor]);
            Ok::<_, Error>(())
        })
    }
    pub(crate) async fn graph_identity_candidate(
        &self,
        rfc_id: &str,
        fingerprint: &str,
        new_id: &str,
    ) -> Result<Option<(String, String)>> {
        let edge = format!("rfc:{rfc_id}");
        // Provider documents also hold opaque cursors, so filter before casting JSON.
        scoped!(self, scope, false, {
            let candidates:Vec<(String,String)>=tuples!(scope,"SELECT m.id,json_extract(r.document,'$.graph_id') FROM thread_edges e JOIN messages m ON m.id=e.message_id JOIN provider_documents h ON h.key='thread-headers:'||m.id JOIN provider_documents r ON r.key='reference:'||m.id WHERE e.edge=? AND json_extract(m.document,'$.provider_id') LIKE 'graph:%' AND json_extract(h.document,'$.fingerprint')=? AND json_extract(r.document,'$.graph_id') IS NOT NULL AND json_extract(r.document,'$.graph_id')<>? LIMIT 2",[&edge,fingerprint,new_id],"WITH candidates AS MATERIALIZED (SELECT m.id,h.document AS headers,r.document AS reference FROM thread_edges e JOIN messages m ON m.account_id=e.account_id AND m.id=e.message_id JOIN provider_documents h ON h.account_id=m.account_id AND h.key='thread-headers:'||m.id JOIN provider_documents r ON r.account_id=m.account_id AND r.key='reference:'||m.id WHERE m.account_id=$1 AND e.edge=$2 AND m.document::jsonb->>'provider_id' LIKE 'graph:%') SELECT id,reference::jsonb->>'graph_id' FROM candidates WHERE headers::jsonb->>'fingerprint'=$3 AND reference::jsonb->>'graph_id' IS NOT NULL AND reference::jsonb->>'graph_id'<>$4 LIMIT 2",[&scope.account,&edge,fingerprint,new_id],fetch_all);
            Ok::<_, Error>(if candidates.len() == 1 {
                candidates.into_iter().next()
            } else {
                None
            })
        })
    }
}

impl AccountRepository {
    pub(crate) async fn snoozed_rows_filtered(
        &self,
        before: Option<(&str, &str)>,
        category: Option<&str>,
        filter: &ClassificationFilter,
    ) -> Result<Vec<Message>> {
        let filter = serde_json::to_string(filter)?;
        let at = now();
        let date = before.map(|b| b.0);
        let id = before.map(|b| b.1);
        transaction!(self, tx, local, false, {
            let sqlite=format!("SELECT m.document FROM snoozes s CROSS JOIN messages m CROSS JOIN (SELECT ? AS filter) cf WHERE s.until>? AND CAST(json_extract(m.document,'$.thread_id') AS TEXT)=s.thread_id AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END {} AND (? IS NULL OR json_extract(m.document,'$.category') IS NULL OR json_extract(m.document,'$.category')=?) AND (? IS NULL OR (json_extract(m.document,'$.received_at'),m.id)<(?,?)) ORDER BY json_extract(m.document,'$.received_at') DESC,m.id DESC LIMIT 51",crate::classification::CLASSIFICATION_SQL);
            let pg=format!("SELECT m.document FROM snoozes s JOIN messages m ON m.account_id=s.account_id AND m.thread_id=s.thread_id CROSS JOIN (SELECT $6::TEXT AS filter) cf WHERE s.account_id=$1 AND s.until>$2 AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=m.account_id AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch) {} AND ($3::TEXT IS NULL OR m.category IS NULL OR m.category=$3) AND ($4::TEXT IS NULL OR (m.received_at,m.id)<($4,$5)) ORDER BY m.received_at DESC,m.id COLLATE \"C\" DESC LIMIT 51",crate::classification::CLASSIFICATION_PG_SQL);
            let rows: Vec<String> = scalar!(
                local,
                tx,
                &sqlite,
                [&filter, &at, category, category, date, date, id],
                &pg,
                [&self.account, &at, category, date, id, &filter],
                fetch_all
            );
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn active_snoozes(&self) -> Result<Vec<serde_json::Value>> {
        let at = now();
        scoped!(self, scope, false, {
            let rows:Vec<(String,String,i64)>=tuples!(scope,"SELECT s.thread_id,s.until,COUNT(m.id) FROM snoozes s CROSS JOIN messages m ON CAST(json_extract(m.document,'$.thread_id') AS TEXT)=s.thread_id WHERE s.until>? AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END GROUP BY s.thread_id,s.until ORDER BY s.until,s.thread_id",[&at],"SELECT s.thread_id,s.until,COUNT(m.id) FROM snoozes s JOIN messages m ON m.account_id=s.account_id AND m.thread_id=s.thread_id WHERE s.account_id=$1 AND s.until>$2 AND NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.account_id=m.account_id AND v.key='thread:epoch:'||m.provider_folder AND v.document::BIGINT<>m.provider_epoch) GROUP BY s.thread_id,s.until ORDER BY s.until,s.thread_id",[&scope.account,&at],fetch_all);
            Ok::<_,Error>(rows.into_iter().map(|(thread,until,count)|serde_json::json!({"thread_id":thread,"until":until,"message_count":count})).collect())
        })
    }
}
