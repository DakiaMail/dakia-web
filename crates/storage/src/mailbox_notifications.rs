use super::*;
const SQLITE_ELIGIBLE:&str="SELECT id FROM messages m WHERE m.id=push_jobs.message_id AND (COALESCE(json_extract(m.document,'$.classification.kind'),'personal')='personal' OR (julianday(json_extract(m.document,'$.received_at')) BETWEEN julianday('now','-30 days') AND julianday('now') AND (json_extract(m.document,'$.classification.attention_override')=1 OR (json_extract(m.document,'$.classification.attention_override') IS NULL AND EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.attention') reason WHERE json_extract(reason.value,'$.expires_at') IS NULL OR json_extract(reason.value,'$.expires_at')>strftime('%Y-%m-%dT%H:%M:%fZ','now')))))) AND (json_extract(m.document,'$.classification.status') IN ('ready','failed') OR json_extract(m.document,'$.classification.source') IN ('message','sender') OR json_extract(m.document,'$.classification.attention_override')=1)";
const PG_ELIGIBLE:&str="SELECT id FROM messages m WHERE m.account_id=push_jobs.account_id AND m.id=push_jobs.message_id AND (COALESCE(m.document::jsonb#>>'{classification,kind}','personal')='personal' OR ((m.document::jsonb#>>'{received_at}')::TIMESTAMPTZ BETWEEN CURRENT_TIMESTAMP-INTERVAL '30 days' AND CURRENT_TIMESTAMP AND ((m.document::jsonb#>>'{classification,attention_override}')::BOOLEAN IS TRUE OR (m.document::jsonb#>>'{classification,attention_override}' IS NULL AND EXISTS(SELECT 1 FROM jsonb_array_elements(m.document::jsonb#>'{classification,attention}') reason WHERE reason->>'expires_at' IS NULL OR reason->>'expires_at'>to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')))))) AND (m.document::jsonb#>>'{classification,status}' IN ('ready','failed') OR m.document::jsonb#>>'{classification,source}' IN ('message','sender') OR (m.document::jsonb#>>'{classification,attention_override}')::BOOLEAN IS TRUE)";
const SQLITE_SUPPRESSED:&str="SELECT id FROM messages m WHERE m.id=push_jobs.message_id AND json_extract(m.document,'$.classification.kind')='automated' AND json_extract(m.document,'$.classification.status') IN ('ready','failed') AND NOT COALESCE((julianday(json_extract(m.document,'$.received_at')) BETWEEN julianday('now','-30 days') AND julianday('now') AND (json_extract(m.document,'$.classification.attention_override')=1 OR (json_extract(m.document,'$.classification.attention_override') IS NULL AND EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.attention') reason WHERE json_extract(reason.value,'$.expires_at') IS NULL OR json_extract(reason.value,'$.expires_at')>strftime('%Y-%m-%dT%H:%M:%fZ','now'))))),0)";
const PG_SUPPRESSED:&str="SELECT id FROM messages m WHERE m.account_id=push_jobs.account_id AND m.id=push_jobs.message_id AND m.document::jsonb#>>'{classification,kind}'='automated' AND m.document::jsonb#>>'{classification,status}' IN ('ready','failed') AND NOT COALESCE(((m.document::jsonb#>>'{received_at}')::TIMESTAMPTZ BETWEEN CURRENT_TIMESTAMP-INTERVAL '30 days' AND CURRENT_TIMESTAMP AND ((m.document::jsonb#>>'{classification,attention_override}')::BOOLEAN IS TRUE OR (m.document::jsonb#>>'{classification,attention_override}' IS NULL AND EXISTS(SELECT 1 FROM jsonb_array_elements(m.document::jsonb#>'{classification,attention}') reason WHERE reason->>'expires_at' IS NULL OR reason->>'expires_at'>to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'))))),FALSE)";

impl MailboxTransaction<'_> {
    async fn suppress_push(&mut self, rules: &[MailRule]) -> Result<()> {
        let sqlite=format!("UPDATE push_jobs SET state='skipped' WHERE state IN ('pending','running') AND (EXISTS ({SQLITE_SUPPRESSED}) OR NOT EXISTS(SELECT 1 FROM messages WHERE id=push_jobs.message_id))");
        let pg=format!("UPDATE push_jobs SET state='skipped' WHERE account_id=$1 AND state IN ('pending','running') AND (EXISTS ({PG_SUPPRESSED}) OR NOT EXISTS(SELECT 1 FROM messages WHERE account_id=$1 AND id=push_jobs.message_id))");
        write!(self, &sqlite, [], &pg, [&self.account]);
        let senders = rules
            .iter()
            .filter(|rule| rule.enabled)
            .map(|rule| {
                let RuleCondition::SenderIs { email } = &rule.condition;
                email.clone()
            })
            .collect::<std::collections::HashSet<_>>();
        if !senders.is_empty() {
            let candidates:Vec<(String,String)>=tuples!(self,"SELECT DISTINCT m.id,json_extract(m.document,'$.sender.email') FROM push_jobs j JOIN messages m ON m.id=j.message_id WHERE j.state IN ('pending','running')",[],"SELECT DISTINCT m.id,m.document::jsonb->'sender'->>'email' FROM push_jobs j JOIN messages m ON m.account_id=j.account_id AND m.id=j.message_id WHERE j.account_id=$1 AND j.state IN ('pending','running')",[&self.account],fetch_all);
            for (id, sender) in candidates {
                if senders.contains(&sender.trim().to_lowercase()) {
                    write!(self,"UPDATE push_jobs SET state='skipped' WHERE message_id=? AND state IN ('pending','running')",[&id],"UPDATE push_jobs SET state='skipped' WHERE account_id=$1 AND message_id=$2 AND state IN ('pending','running')",[&self.account,&id]);
                }
            }
        }
        Ok(())
    }
    async fn next_push(&mut self, rules: &[MailRule]) -> Result<Option<(serde_json::Value, i64)>> {
        self.suppress_push(rules).await?;
        let deadline = (Utc::now() + Duration::minutes(5)).to_rfc3339();
        let at = Utc::now().timestamp();
        // An abandoned send may have reached the push service. Never replay it.
        write!(self,"UPDATE push_jobs SET state='done' WHERE rowid IN (SELECT rowid FROM push_jobs WHERE state IN ('pending','running') AND state='running' AND unixepoch(json_extract(document,'$.deadline'))<=? ORDER BY rowid LIMIT 200)",[at],"UPDATE push_jobs SET state='done' WHERE account_id=$1 AND id IN (SELECT id FROM push_jobs WHERE account_id=$1 AND state='running' AND floor(extract(epoch FROM (document::jsonb->>'deadline')::TIMESTAMPTZ))::BIGINT<=$2 ORDER BY id LIMIT 200)",[&self.account,at]);
        let sqlite=format!("UPDATE push_jobs SET state='running',document=CASE WHEN attempts=0 THEN json_set(document,'$.deadline',?) ELSE document END,attempts=attempts+1 WHERE rowid=(SELECT rowid FROM push_jobs WHERE state IN ('pending','running') AND state='pending' AND next_attempt<=? AND attempts<5 AND EXISTS ({SQLITE_ELIGIBLE}) ORDER BY rowid LIMIT 1) RETURNING document,attempts");
        let pg=format!("UPDATE push_jobs SET state='running',document=CASE WHEN attempts=0 THEN jsonb_set(document::jsonb,'{{deadline}}',to_jsonb($2::TEXT))::TEXT ELSE document END,attempts=attempts+1 WHERE account_id=$1 AND id=(SELECT id FROM push_jobs WHERE account_id=$1 AND state='pending' AND next_attempt<=$3 AND attempts<5 AND EXISTS ({PG_ELIGIBLE}) ORDER BY id LIMIT 1) RETURNING document,attempts");
        let raw: Option<(String, i64)> = tuples!(
            self,
            &sqlite,
            [&deadline, at],
            &pg,
            [&self.account, &deadline, at],
            fetch_optional
        );
        raw.map(|(s, attempt)| Ok((serde_json::from_str(&s)?, attempt)))
            .transpose()
    }
    async fn push_send_allowed(
        &mut self,
        message: &str,
        subscription: &str,
        rules: &[MailRule],
    ) -> Result<bool> {
        self.suppress_push(rules).await?;
        let sqlite=format!("SELECT EXISTS(SELECT 1 FROM push_jobs WHERE message_id=? AND subscription_id=? AND state='running' AND EXISTS ({SQLITE_ELIGIBLE}))");
        let pg=format!("SELECT EXISTS(SELECT 1 FROM push_jobs WHERE account_id=$1 AND message_id=$2 AND subscription_id=$3 AND state='running' AND EXISTS ({PG_ELIGIBLE}))");
        let allowed: bool = value!(
            self,
            &sqlite,
            [message, subscription],
            &pg,
            [&self.account, message, subscription],
            fetch_one
        );
        if !allowed {
            write!(self,"UPDATE push_jobs SET state='pending',attempts=MAX(0,attempts-1) WHERE message_id=? AND subscription_id=? AND state='running'",[message,subscription],"UPDATE push_jobs SET state='pending',attempts=GREATEST(0,attempts-1) WHERE account_id=$1 AND message_id=$2 AND subscription_id=$3 AND state='running'",[&self.account,message,subscription]);
        }
        Ok(allowed)
    }
    async fn finish_push(
        &mut self,
        message: &str,
        subscription: &str,
        retry: Option<u32>,
        attempt: Option<i64>,
    ) -> Result<()> {
        let state = if retry.is_some() { "pending" } else { "done" };
        let at = Utc::now().timestamp() + i64::from(retry.unwrap_or(0).min(3600));
        write!(self,"UPDATE push_jobs SET state=?,next_attempt=? WHERE message_id=? AND subscription_id=? AND state='running' AND (? IS NULL OR attempts=?)",[state,at,message,subscription,attempt,attempt],"UPDATE push_jobs SET state=$1,next_attempt=$2 WHERE account_id=$3 AND message_id=$4 AND subscription_id=$5 AND state='running' AND ($6::BIGINT IS NULL OR attempts=$6)",[state,at,&self.account,message,subscription,attempt]);
        Ok(())
    }
    async fn recover_push(&mut self, rules: &[MailRule]) -> Result<()> {
        self.suppress_push(rules).await?;
        write!(
            self,
            "UPDATE push_jobs SET state='pending' WHERE state='running'",
            [],
            "UPDATE push_jobs SET state='pending' WHERE account_id=$1 AND state='running'",
            [&self.account]
        );
        Ok(())
    }
    async fn wake_snoozes(&mut self) -> Result<()> {
        let at = now();
        let threads: Vec<String> = value!(
            self,
            "DELETE FROM snoozes WHERE until<=? RETURNING thread_id",
            [&at],
            "DELETE FROM snoozes WHERE account_id=$1 AND until<=$2 RETURNING thread_id",
            [&self.account, &at],
            fetch_all
        );
        for thread in threads {
            self.event(0, "snooze_woke", &thread).await?;
        }
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn next_push(&self, rules: &[MailRule]) -> Result<Option<serde_json::Value>> {
        Ok(self.next_push_claim(rules).await?.map(|(job, _)| job))
    }
    pub(crate) async fn next_push_claim(
        &self,
        rules: &[MailRule],
    ) -> Result<Option<(serde_json::Value, i64)>> {
        scoped!(self, scope, true, { scope.next_push(rules).await })
    }
    pub(crate) async fn push_send_allowed(
        &self,
        message: &str,
        subscription: &str,
        rules: &[MailRule],
    ) -> Result<bool> {
        scoped!(self, scope, true, {
            scope.push_send_allowed(message, subscription, rules).await
        })
    }
    pub(crate) async fn finish_push(
        &self,
        message: &str,
        subscription: &str,
        retry: Option<u32>,
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope.finish_push(message, subscription, retry, None).await
        })
    }
    pub(crate) async fn finish_push_attempt(
        &self,
        message: &str,
        subscription: &str,
        attempt: i64,
        retry: Option<u32>,
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope
                .finish_push(message, subscription, retry, Some(attempt))
                .await
        })
    }
    pub(crate) async fn recover_push(&self, rules: &[MailRule]) -> Result<()> {
        scoped!(self, scope, true, { scope.recover_push(rules).await })
    }
    pub(crate) async fn is_snoozed(&self, thread: &str) -> Result<bool> {
        let at = now();
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM snoozes WHERE thread_id=? AND until>?)",[thread,&at],"SELECT EXISTS(SELECT 1 FROM snoozes WHERE account_id=$1 AND thread_id=$2 AND until>$3)",[&self.account,thread,&at],fetch_one))
        })
    }
    pub(crate) async fn wake_snoozes(&self) -> Result<()> {
        scoped!(self, scope, true, { scope.wake_snoozes().await })
    }
}
