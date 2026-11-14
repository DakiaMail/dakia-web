use super::*;

impl MailboxTransaction<'_> {
    pub(crate) async fn cancel_rule(&mut self, id: &str) -> Result<()> {
        write!(self,"UPDATE rule_executions SET state='cancelled' WHERE rule_id=? AND state='queued' AND operation_id IN (SELECT target_id FROM outbox WHERE kind='mutation' AND state='pending')",[id],"UPDATE rule_executions SET state='cancelled' WHERE account_id=$1 AND rule_id=$2 AND state='queued' AND operation_id IN (SELECT target_id FROM outbox WHERE account_id=$1 AND kind='mutation' AND state='pending')",[&self.account,id]);
        write!(self,"UPDATE operations SET document=json_set(document,'$.status','failed') WHERE id IN (SELECT operation_id FROM rule_executions WHERE state='cancelled') AND json_extract(document,'$.status')='pending'",[],"UPDATE operations SET document=jsonb_set(document::jsonb,'{status}','\"failed\"')::TEXT WHERE account_id=$1 AND id IN (SELECT operation_id FROM rule_executions WHERE account_id=$1 AND state='cancelled') AND status='pending'",[&self.account]);
        write!(self,"UPDATE outbox SET state='done' WHERE kind='mutation' AND state='pending' AND target_id IN (SELECT operation_id FROM rule_executions WHERE state='cancelled')",[],"UPDATE outbox SET state='done' WHERE account_id=$1 AND kind='mutation' AND state='pending' AND target_id IN (SELECT operation_id FROM rule_executions WHERE account_id=$1 AND state='cancelled')",[&self.account]);
        Ok(())
    }
    async fn claim_mutation(&mut self, active_rules: &[String]) -> Result<Option<Operation>> {
        let queued: Vec<String> = value!(
            self,
            "SELECT DISTINCT rule_id FROM rule_executions WHERE state='queued'",
            [],
            "SELECT DISTINCT rule_id FROM rule_executions WHERE account_id=$1 AND state='queued'",
            [&self.account],
            fetch_all
        );
        for id in queued {
            if !active_rules.contains(&id) {
                self.cancel_rule(&id).await?;
            }
        }
        let at = now();
        let target:Option<String>=value!(self,"UPDATE outbox SET state='running' WHERE id=(SELECT q.id FROM outbox q JOIN operations o ON o.id=q.target_id WHERE q.kind='mutation' AND q.state='pending' AND (q.lease_until IS NULL OR q.lease_until<=?) AND NOT EXISTS(SELECT 1 FROM outbox prior JOIN operations p ON p.id=prior.target_id WHERE prior.kind='mutation' AND prior.state IN ('pending','running') AND prior.id<q.id AND json_extract(p.document,'$.target_id')=json_extract(o.document,'$.target_id')) ORDER BY q.id LIMIT 1) AND state='pending' RETURNING target_id",[&at],"UPDATE outbox SET state='running' WHERE account_id=$1 AND id=(SELECT q.id FROM outbox q JOIN operations o ON o.account_id=q.account_id AND o.id=q.target_id WHERE q.account_id=$1 AND q.kind='mutation' AND q.state='pending' AND (q.lease_until IS NULL OR q.lease_until<=$2) AND NOT EXISTS(SELECT 1 FROM outbox prior JOIN operations p ON p.account_id=prior.account_id AND p.id=prior.target_id WHERE prior.account_id=$1 AND prior.kind='mutation' AND prior.state IN ('pending','running') AND prior.id<q.id AND p.target_id=o.target_id) ORDER BY q.id LIMIT 1) AND state='pending' RETURNING target_id",[&self.account,&at],fetch_optional);
        let Some(target) = target else {
            return Ok(None);
        };
        let raw: String = value!(
            self,
            "SELECT document FROM operations WHERE id=?",
            [&target],
            "SELECT document FROM operations WHERE account_id=$1 AND id=$2",
            [&self.account, &target],
            fetch_one
        );
        Ok(Some(serde_json::from_str(&raw)?))
    }
    async fn retry_mutation(&mut self, id: &str) -> Result<bool> {
        let key = format!("mutation-retry:{id}");
        let attempts: Option<String> = value!(
            self,
            "SELECT document FROM provider_documents WHERE key=?",
            [&key],
            "SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2",
            [&self.account, &key],
            fetch_optional
        );
        let attempts = attempts
            .map(|s| s.parse::<u32>().map_err(|_| Error::Invalid))
            .transpose()?
            .unwrap_or(0);
        if attempts >= 3 {
            return Ok(false);
        }
        let due = (Utc::now() + Duration::seconds(5 * (1 << attempts)))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let changed=write!(self,"UPDATE outbox SET state='pending',lease_until=? WHERE kind='mutation' AND target_id=? AND state='running'",[&due,id],"UPDATE outbox SET state='pending',lease_until=$1 WHERE account_id=$2 AND kind='mutation' AND target_id=$3 AND state='running'",[&due,&self.account,id]);
        if changed != 1 {
            return Err(Error::Conflict);
        }
        let attempts = (attempts + 1).to_string();
        write!(self,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document",[&key,&attempts],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&self.account,&key,&attempts]);
        Ok(true)
    }
    async fn finish_mutation(
        &mut self,
        mut operation: Operation,
        folders: Option<Vec<String>>,
    ) -> Result<()> {
        if operation.account_id != self.account {
            return Err(Error::Invalid);
        }
        write!(
            self,
            "UPDATE operations SET document=document WHERE id=?",
            [&operation.id],
            "UPDATE operations SET document=document WHERE account_id=$1 AND id=$2",
            [&self.account, &operation.id]
        );
        let mut message = self.message(&operation.target_id).await?;
        if let Some(folders) = folders {
            message.folder_ids = folders;
        }
        let restore = format!("snooze-restore:{}", operation.id);
        if operation.status == OperationStatus::Applied
            && matches!(&operation.change,Change::Move { folder_id } if folder_id.eq_ignore_ascii_case("INBOX"))
        {
            write!(self,"DELETE FROM snoozes WHERE thread_id=? AND EXISTS(SELECT 1 FROM provider_documents WHERE key=? AND document=snoozes.thread_id)",[&message.thread_id,&restore],"DELETE FROM snoozes s WHERE account_id=$1 AND thread_id=$2 AND EXISTS(SELECT 1 FROM provider_documents p WHERE p.account_id=s.account_id AND p.key=$3 AND p.document=s.thread_id)",[&self.account,&message.thread_id,&restore]);
        }
        message.revision = message.revision.checked_add(1).ok_or(Error::Invalid)?;
        operation.revision = message.revision;
        self.store_message(&message).await?;
        let raw = serde_json::to_string(&operation)?;
        write!(
            self,
            "UPDATE operations SET document=? WHERE id=?",
            [&raw, &operation.id],
            "UPDATE operations SET document=$1 WHERE account_id=$2 AND id=$3",
            [&raw, &self.account, &operation.id]
        );
        write!(self,"UPDATE outbox SET state='done' WHERE kind='mutation' AND target_id=?",[&operation.id],"UPDATE outbox SET state='done' WHERE account_id=$1 AND kind='mutation' AND target_id=$2",[&self.account,&operation.id]);
        write!(
            self,
            "DELETE FROM provider_documents WHERE key=?",
            [&restore],
            "DELETE FROM provider_documents WHERE account_id=$1 AND key=$2",
            [&self.account, &restore]
        );
        let key = format!("mutation-retry:{}", operation.id);
        write!(
            self,
            "DELETE FROM provider_documents WHERE key=?",
            [&key],
            "DELETE FROM provider_documents WHERE account_id=$1 AND key=$2",
            [&self.account, &key]
        );
        self.event(
            operation.revision,
            "operation_changed",
            &operation.target_id,
        )
        .await?;
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn rule_changed(&self, rule: &MailRule) -> Result<()> {
        scoped!(self, scope, true, {
            if !rule.enabled {
                scope.cancel_rule(&rule.id).await?;
            }
            scope.event(0, "rules_changed", &rule.id).await?;
            Ok::<_, Error>(())
        })
    }
    pub(crate) async fn rule_activity(&self) -> Result<Vec<RuleActivity>> {
        transaction!(self, tx, local, false, {
            let rows: Vec<(String, i64, i64)> = if local {
                sqlx::query_as("SELECT e.rule_id,SUM(CASE WHEN e.state='queued' AND json_extract(o.document,'$.status')='pending' THEN 1 ELSE 0 END),SUM(CASE WHEN e.state='conflict' OR (e.state='queued' AND json_extract(o.document,'$.status')='failed') THEN 1 ELSE 0 END) FROM rule_executions e LEFT JOIN operations o ON o.id=e.operation_id WHERE e.state<>'cancelled' GROUP BY e.rule_id").fetch_all(&mut *tx).await?
            } else {
                sqlx::query_as("SELECT e.rule_id,SUM(CASE WHEN e.state='queued' AND o.status='pending' THEN 1 ELSE 0 END),SUM(CASE WHEN e.state='conflict' OR (e.state='queued' AND o.status='failed') THEN 1 ELSE 0 END) FROM rule_executions e LEFT JOIN operations o ON o.account_id=e.account_id AND o.id=e.operation_id WHERE e.account_id=$1 AND e.state<>'cancelled' GROUP BY e.rule_id").bind(&self.account).fetch_all(&mut *tx).await?
            };
            rows.into_iter()
                .map(|(rule_id, pending, failed)| {
                    Ok(RuleActivity {
                        rule_id,
                        pending: u32::try_from(pending).map_err(|_| Error::Invalid)?,
                        failed: u32::try_from(failed).map_err(|_| Error::Invalid)?,
                    })
                })
                .collect()
        })
    }
    pub(crate) async fn rule_executions(&self) -> Result<Vec<RuleExecution>> {
        transaction!(self, tx, local, false, {
            let rows: Vec<(String, String, Option<String>, String)> = if local {
                sqlx::query_as("SELECT e.rule_id,e.message_id,e.operation_id,CASE WHEN e.state='queued' THEN COALESCE(json_extract(o.document,'$.status'),'failed') ELSE e.state END FROM rule_executions e LEFT JOIN operations o ON o.id=e.operation_id WHERE e.state<>'cancelled' AND (e.state='conflict' OR json_extract(o.document,'$.status') IN ('pending','failed')) ORDER BY e.rowid DESC LIMIT 200").fetch_all(&mut *tx).await?
            } else {
                sqlx::query_as("SELECT e.rule_id,e.message_id,e.operation_id,CASE WHEN e.state='queued' THEN COALESCE(o.status,'failed') ELSE e.state END FROM rule_executions e LEFT JOIN operations o ON o.account_id=e.account_id AND o.id=e.operation_id WHERE e.account_id=$1 AND e.state<>'cancelled' AND (e.state='conflict' OR o.status IN ('pending','failed')) ORDER BY e.id DESC LIMIT 200").bind(&self.account).fetch_all(&mut *tx).await?
            };
            Ok(rows
                .into_iter()
                .map(|(rule_id, message_id, operation_id, state)| RuleExecution {
                    rule_id,
                    message_id,
                    operation_id,
                    state,
                })
                .collect())
        })
    }
    pub(crate) async fn claim_mutation(
        &self,
        active_rules: &[String],
    ) -> Result<Option<Operation>> {
        scoped!(self, scope, true, {
            scope.claim_mutation(active_rules).await
        })
    }
    pub(crate) async fn retry_mutation(&self, id: &str) -> Result<bool> {
        scoped!(self, scope, true, { scope.retry_mutation(id).await })
    }
    pub(crate) async fn finish_mutation(
        &self,
        operation: Operation,
        folders: Option<Vec<String>>,
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope.finish_mutation(operation, folders).await
        })
    }
    pub(crate) async fn recover_mutations(&self) -> Result<()> {
        scoped!(self, scope, true, {
            let rows:Vec<String>=value!(scope,"SELECT operations.document FROM operations JOIN outbox ON outbox.target_id=operations.id WHERE outbox.kind='mutation' AND outbox.state='running'",[],"SELECT o.document FROM operations o JOIN outbox q ON q.account_id=o.account_id AND q.target_id=o.id WHERE o.account_id=$1 AND q.kind='mutation' AND q.state='running'",[&self.account],fetch_all);
            for raw in rows {
                let mut operation: Operation = serde_json::from_str(&raw)?;
                operation.status = OperationStatus::Failed;
                scope.finish_mutation(operation, None).await?;
            }
            Ok::<_, Error>(())
        })
    }
    pub(crate) async fn link_identity(&self, provider: &str, message: &str) -> Result<()> {
        transaction!(self, tx, local, true, {
            execute!(local,tx,"INSERT INTO message_identities(provider_id,message_id) VALUES(?,?) ON CONFLICT(provider_id) DO UPDATE SET message_id=excluded.message_id",[provider,message],"INSERT INTO message_identities(account_id,provider_id,message_id) VALUES($1,$2,$3) ON CONFLICT(account_id,provider_id) DO UPDATE SET message_id=excluded.message_id",[&self.account,provider,message]);
            tx.commit().await?;
            Ok(())
        })
    }
}
