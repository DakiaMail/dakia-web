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
        self.control_repository().rules(user).await
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
        let old = self
            .control_repository()
            .rule_by_sender(user, &email)
            .await?;
        let mut rule = if let Some(rule) = old {
            rule
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
        rule = self
            .control_repository()
            .upsert_rule(user, &email, &rule)
            .await?;
        self.rule_changed(user, &rule).await?;
        Ok(rule)
    }

    pub async fn update_rule(&self, user: &str, id: &str, update: UpdateRule) -> Result<MailRule> {
        let _guard = self.gate.read().await;
        let _rules = self.rules_gate.write().await;
        let mut rule = self.control_repository().rule(user, id).await?;
        if update.enabled && !rule.enabled {
            self.rule_changed(user, &rule).await?;
            rule.activated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true);
        }
        rule.enabled = update.enabled;
        self.control_repository().update_rule(user, &rule).await?;
        self.rule_changed(user, &rule).await?;
        Ok(rule)
    }

    async fn rule_changed(&self, user: &str, rule: &MailRule) -> Result<()> {
        for account in self.accounts(user).await? {
            let a = self.authorize(user, &account.id).await?;
            self.account_repository(&a)
                .await?
                .rule_changed(rule)
                .await?;
        }
        Ok(())
    }
    pub async fn rule_activity(&self, a: &AuthorizedAccount) -> Result<Vec<RuleActivity>> {
        self.account_repository(a).await?.rule_activity().await
    }
    pub async fn rule_executions(&self, a: &AuthorizedAccount) -> Result<Vec<RuleExecution>> {
        self.account_repository(a).await?.rule_executions().await
    }
}
