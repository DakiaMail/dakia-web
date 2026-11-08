//! Shared matching and durable execution. No provider work is performed here.
use super::*;

pub fn normalize_rule_sender(raw: &str) -> Result<String> {
    let email = raw.trim().to_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return Err(Error::Invalid);
    };
    if email.len() > 320
        || local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>\"(),;:".contains(c))
    {
        return Err(Error::Invalid);
    }
    Ok(email)
}

/// Predicate only. Folder eligibility and first-arrival receipts belong to the
/// executor, so future presets can reuse matching without triggering writes.
pub fn rule_matches(rule: &MailRule, message: &Message) -> bool {
    let RuleCondition::SenderIs { email } = &rule.condition;
    rule.enabled
        && message.sender.email.trim().to_lowercase() == *email
        && DateTime::parse_from_rfc3339(&message.received_at)
            .ok()
            .zip(DateTime::parse_from_rfc3339(&rule.activated_at).ok())
            .is_some_and(|(received, activated)| received > activated)
}

impl Store {
    pub async fn rules(&self, user: &str) -> Result<Vec<MailRule>> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT document FROM mail_rules WHERE user_id=? ORDER BY sender")
                .bind(user)
                .fetch_all(&self.control)
                .await?;
        rows.into_iter()
            .map(|raw| Ok(serde_json::from_str(&raw)?))
            .collect()
    }

    pub async fn create_rule(&self, user: &str, request: CreateRule) -> Result<MailRule> {
        let _guard = self.gate.read().await;
        let _rules = self.rules_gate.write().await;
        // Closed preset contract: neither an omitted suppression action nor a
        // duplicate/unsupported action can silently change block semantics.
        if request.actions.len() != 2
            || !request.actions.contains(&RuleAction::Trash)
            || !request.actions.contains(&RuleAction::SuppressNotifications)
        {
            return Err(Error::Invalid);
        }
        let RuleCondition::SenderIs { email } = request.condition;
        let email = normalize_rule_sender(&email)?;
        let old: Option<String> = sqlx::query_scalar("SELECT document FROM mail_rules WHERE user_id=? AND preset='block_sender' AND sender=?")
            .bind(user).bind(&email).fetch_optional(&self.control).await?;
        let mut rule = if let Some(raw) = old {
            serde_json::from_str::<MailRule>(&raw)?
        } else {
            MailRule {
                id: Uuid::new_v4().to_string(),
                preset: request.preset,
                enabled: false,
                activated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true),
                scope: request.scope,
                condition: RuleCondition::SenderIs {
                    email: email.clone(),
                },
                actions: vec![RuleAction::Trash, RuleAction::SuppressNotifications],
            }
        };
        if !rule.enabled {
            self.rule_changed(user, &rule).await?;
            rule.enabled = true;
            rule.activated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true);
        }
        sqlx::query("INSERT INTO mail_rules(id,user_id,preset,sender,document) VALUES(?,?,'block_sender',?,?) ON CONFLICT(user_id,preset,sender) DO UPDATE SET document=excluded.document")
            .bind(&rule.id).bind(user).bind(email).bind(serde_json::to_string(&rule)?).execute(&self.control).await?;
        self.rule_changed(user, &rule).await?;
        Ok(rule)
    }

    pub async fn update_rule(&self, user: &str, id: &str, update: UpdateRule) -> Result<MailRule> {
        let _guard = self.gate.read().await;
        let _rules = self.rules_gate.write().await;
        let raw: Option<String> =
            sqlx::query_scalar("SELECT document FROM mail_rules WHERE user_id=? AND id=?")
                .bind(user)
                .bind(id)
                .fetch_optional(&self.control)
                .await?;
        let mut rule: MailRule = serde_json::from_str(&raw.ok_or(Error::NotFound)?)?;
        if update.enabled && !rule.enabled {
            self.rule_changed(user, &rule).await?;
            rule.activated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true);
        }
        rule.enabled = update.enabled;
        sqlx::query("UPDATE mail_rules SET document=? WHERE user_id=? AND id=?")
            .bind(serde_json::to_string(&rule)?)
            .bind(user)
            .bind(id)
            .execute(&self.control)
            .await?;
        self.rule_changed(user, &rule).await?;
        Ok(rule)
    }

    async fn rule_changed(&self, user: &str, rule: &MailRule) -> Result<()> {
        for account in self.accounts(user).await? {
            let a = self.authorize(user, &account.id).await?;
            let db = self.db(&a).await?;
            let mut tx = db.begin().await?;
            if !rule.enabled {
                Self::cancel_rule_tx(&mut tx, &rule.id).await?;
            }
            sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(0,'rules_changed',?)")
                .bind(&rule.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(())
    }

    async fn cancel_rule_tx(tx: &mut sqlx::SqliteConnection, id: &str) -> Result<()> {
        // A claim is the start boundary. Running/uncertain operations are never
        // cancelled or replayed, even if the rule was just disabled.
        sqlx::query("UPDATE rule_executions SET state='cancelled' WHERE rule_id=? AND state='queued' AND operation_id IN (SELECT target_id FROM outbox WHERE kind='mutation' AND state='pending')")
            .bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE operations SET document=json_set(document,'$.status','failed') WHERE id IN (SELECT operation_id FROM rule_executions WHERE state='cancelled') AND json_extract(document,'$.status')='pending'")
            .execute(&mut *tx).await?;
        sqlx::query("UPDATE outbox SET state='done' WHERE kind='mutation' AND state='pending' AND target_id IN (SELECT operation_id FROM rule_executions WHERE state='cancelled')")
            .execute(&mut *tx).await?;
        Ok(())
    }

    /// Also used at claim time to complete cancellation after an interrupted
    /// cross-database update. The control database is the source of truth.
    pub(super) async fn cancel_disabled_rules_tx(
        &self,
        tx: &mut sqlx::SqliteConnection,
        a: &AuthorizedAccount,
    ) -> Result<()> {
        let active: Vec<String> = self
            .rules(&a.user_id)
            .await?
            .into_iter()
            .filter(|r| r.enabled)
            .map(|r| r.id)
            .collect();
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT rule_id FROM rule_executions WHERE state='queued'")
                .fetch_all(&mut *tx)
                .await?;
        for id in ids {
            if !active.contains(&id) {
                Self::cancel_rule_tx(tx, &id).await?;
            }
        }
        Ok(())
    }

    pub async fn rule_activity(&self, a: &AuthorizedAccount) -> Result<Vec<RuleActivity>> {
        let db = self.db(a).await?;
        let rows = sqlx::query("SELECT e.rule_id, SUM(CASE WHEN e.state='queued' AND json_extract(o.document,'$.status')='pending' THEN 1 ELSE 0 END) AS pending, SUM(CASE WHEN e.state='conflict' OR (e.state='queued' AND json_extract(o.document,'$.status')='failed') THEN 1 ELSE 0 END) AS failed FROM rule_executions e LEFT JOIN operations o ON o.id=e.operation_id WHERE e.state<>'cancelled' GROUP BY e.rule_id")
            .fetch_all(&db).await?;
        Ok(rows
            .into_iter()
            .map(|row| RuleActivity {
                rule_id: row.get("rule_id"),
                pending: row.get("pending"),
                failed: row.get("failed"),
            })
            .collect())
    }

    pub async fn rule_executions(&self, a: &AuthorizedAccount) -> Result<Vec<RuleExecution>> {
        let db = self.db(a).await?;
        let rows = sqlx::query("SELECT e.rule_id,e.message_id,e.operation_id,CASE WHEN e.state='queued' THEN COALESCE(json_extract(o.document,'$.status'),'failed') ELSE e.state END AS state FROM rule_executions e LEFT JOIN operations o ON o.id=e.operation_id WHERE e.state<>'cancelled' AND (e.state='conflict' OR json_extract(o.document,'$.status') IN ('pending','failed')) ORDER BY e.rowid DESC LIMIT 200")
            .fetch_all(&db).await?;
        Ok(rows
            .into_iter()
            .map(|r| RuleExecution {
                rule_id: r.get("rule_id"),
                message_id: r.get("message_id"),
                operation_id: r.get("operation_id"),
                state: r.get("state"),
            })
            .collect())
    }

    pub(super) async fn evaluate_rules_tx(
        &self,
        tx: &mut sqlx::SqliteConnection,
        a: &AuthorizedAccount,
        message: &mut Message,
        already_seen: bool,
        rules: &[MailRule],
    ) -> Result<()> {
        let inbox = message
            .folder_ids
            .iter()
            .any(|f| f.trim_start_matches('\\').eq_ignore_ascii_case("inbox"));
        if !inbox {
            return Ok(());
        }
        let first =
            sqlx::query("INSERT OR IGNORE INTO rule_inbox_observations(message_id) VALUES(?)")
                .bind(&message.id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                == 1;
        if !first || already_seen {
            return Ok(());
        }
        if !rules.iter().any(|rule| rule_matches(rule, message)) {
            return Ok(());
        }
        let mut excluded = vec![
            "trash".to_string(),
            "sent".into(),
            "drafts".into(),
            "draft".into(),
        ];
        let docs: Vec<String> = sqlx::query_scalar(
            "SELECT document FROM provider_documents WHERE key IN ('mailbox-settings','probe')",
        )
        .fetch_all(&mut *tx)
        .await?;
        for raw in docs {
            let doc: serde_json::Value = serde_json::from_str(&raw)?;
            for field in ["trash_folder", "sent_folder"] {
                if let Some(folder) = doc[field].as_str() {
                    excluded.push(folder.to_lowercase());
                }
            }
            if let Some(folders) = doc["folders"].as_array() {
                for folder in folders {
                    if folder["attributes"].as_array().is_some_and(|attrs| {
                        attrs.iter().any(|v| {
                            v.as_str().is_some_and(|s| {
                                ["trash", "sent", "drafts"]
                                    .contains(&s.trim_start_matches('\\').to_lowercase().as_str())
                            })
                        })
                    }) {
                        if let Some(name) = folder["name"].as_str() {
                            excluded.push(name.to_lowercase());
                        }
                    }
                }
            }
        }
        if message
            .folder_ids
            .iter()
            .any(|f| excluded.contains(&f.trim_start_matches('\\').to_lowercase()))
        {
            return Ok(());
        }
        let connected: Option<String> =
            sqlx::query_scalar("SELECT connected_at FROM accounts WHERE id=? AND user_id=?")
                .bind(&a.id)
                .bind(&a.user_id)
                .fetch_one(&self.control)
                .await?;
        if connected
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .is_some_and(|at| {
                DateTime::parse_from_rfc3339(&message.received_at)
                    .map_or(true, |received| received <= at)
            })
        {
            return Ok(());
        }
        for rule in rules {
            if !rule_matches(rule, message) {
                continue;
            }
            let conflict: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM operations WHERE json_extract(document,'$.target_id')=? AND json_extract(document,'$.status') IN ('pending','failed') AND json_extract(document,'$.change.kind') IN ('archive','trash','spam','move') AND id NOT IN (SELECT operation_id FROM rule_executions WHERE state='cancelled' AND operation_id IS NOT NULL))")
                .bind(&message.id).fetch_one(&mut *tx).await?;
            let id = format!("rule:{}:{}", rule.id, message.id);
            let inserted = sqlx::query("INSERT OR IGNORE INTO rule_executions(rule_id,message_id,operation_id,state) VALUES(?,?,?,?)")
                .bind(&rule.id).bind(&message.id).bind((!conflict).then_some(&id)).bind(if conflict { "conflict" } else { "queued" }).execute(&mut *tx).await?;
            if inserted.rows_affected() == 0 || conflict {
                continue;
            }
            let operation = Self::operation_tx(
                tx,
                a,
                Operation {
                    id,
                    account_id: a.id.clone(),
                    target_id: message.id.clone(),
                    expected_revision: message.revision,
                    change: Change::Trash,
                    status: OperationStatus::Pending,
                    revision: 0,
                },
            )
            .await?;
            message.revision = operation.revision;
        }
        Ok(())
    }

    pub(super) async fn suppress_rule_push_tx(
        &self,
        tx: &mut sqlx::SqliteConnection,
        a: &AuthorizedAccount,
    ) -> Result<()> {
        let senders: std::collections::HashSet<String> = self
            .rules(&a.user_id)
            .await?
            .into_iter()
            .filter(|rule| rule.enabled)
            .map(|rule| {
                let RuleCondition::SenderIs { email } = rule.condition;
                email
            })
            .collect();
        if senders.is_empty() {
            return Ok(());
        }
        // Rust normalization is shared with matching, including international
        // addresses. SQLite lower() only folds ASCII.
        let candidates = sqlx::query("SELECT DISTINCT m.id, json_extract(m.document,'$.sender.email') AS sender FROM push_jobs j JOIN messages m ON m.id=j.message_id WHERE j.state IN ('pending','running')")
            .fetch_all(&mut *tx).await?;
        for row in candidates {
            if senders.contains(&row.get::<String, _>("sender").trim().to_lowercase()) {
                sqlx::query("UPDATE push_jobs SET state='skipped' WHERE message_id=? AND state IN ('pending','running')")
                    .bind(row.get::<String,_>("id")).execute(&mut *tx).await?;
            }
        }
        Ok(())
    }
}
