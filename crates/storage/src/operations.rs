use super::*;
impl Store {
    pub async fn operation_status(&self, a: &AuthorizedAccount, id: &str) -> Result<Operation> {
        let db = self.db(a).await?;
        let raw: Option<String> = sqlx::query_scalar("SELECT document FROM operations WHERE id=?")
            .bind(id)
            .fetch_optional(&db)
            .await?;
        Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
    }

    /// Project queued intent without persisting invented provider memberships.
    /// The worker always reads the original provider location with message().
    pub async fn visible_message(
        &self,
        a: &AuthorizedAccount,
        mut message: Message,
    ) -> Result<Message> {
        let db = self.db(a).await?;
        let pending: Option<String> = sqlx::query_scalar("SELECT json_extract(document,'$.change.kind') FROM operations WHERE json_extract(document,'$.target_id')=? AND json_extract(document,'$.status')='pending' AND json_extract(document,'$.change.kind') IN ('archive','spam','trash') ORDER BY json_extract(document,'$.change.kind')='trash' DESC LIMIT 1")
            .bind(&message.id).fetch_optional(&db).await?;
        if pending.as_deref() == Some("trash") {
            message.folder_ids = vec!["Trash".into()];
            return Ok(message);
        }
        if pending.is_some() {
            message
                .folder_ids
                .retain(|f| !f.eq_ignore_ascii_case("INBOX") && f != "\\Inbox");
        }
        Ok(message)
    }

    pub async fn upsert_message(&self, a: &AuthorizedAccount, message: Message) -> Result<Message> {
        let _guard = self.gate.read().await;
        if message.account_id != a.id {
            return Err(Error::Invalid);
        }
        let _rules_guard = self.rules_gate.read().await;
        let rules = self.rules(&a.user_id).await?;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let known = Self::previous_inbox_tx(&mut tx, &message.id).await?;
        let mut message = Self::upsert_message_tx(&mut tx, message).await?;
        self.evaluate_rules_tx(&mut tx, a, &mut message, known, &rules)
            .await?;
        tx.commit().await?;
        Ok(message)
    }

    /// One commit per page, retaining revisions, categories and pending intents.
    pub async fn ingest_messages(
        &self,
        a: &AuthorizedAccount,
        messages: Vec<Message>,
        documents: &[(String, String)],
    ) -> Result<Vec<Message>> {
        let _guard = self.gate.read().await;
        // Historical acquisition is capped at 200, with a separate byte budget
        // so large documents cannot turn a count-only limit into unbounded work.
        if messages.len() > 200
            || messages.iter().any(|m| m.account_id != a.id)
            || documents.len() > 1000
            || documents
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
        {
            return Err(Error::Invalid);
        }
        let _rules_guard = self.rules_gate.read().await;
        let rules = self.rules(&a.user_id).await?;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let mut rows = Vec::with_capacity(messages.len());
        let mut known = Vec::with_capacity(messages.len());
        for message in messages {
            known.push(Self::previous_inbox_tx(&mut tx, &message.id).await?);
            rows.push(Self::upsert_message_tx(&mut tx, message).await?);
        }
        for (key, document) in documents {
            if key.starts_with("__content_observation:") {
                continue;
            }
            sqlx::query("INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE document<>excluded.document")
                .bind(key).bind(document).execute(&mut *tx).await?;
        }
        for (row, known) in rows.iter_mut().zip(known) {
            self.evaluate_rules_tx(&mut tx, a, row, known, &rules)
                .await?;
            if let Some((_, reference)) = documents
                .iter()
                .rev()
                .find(|(key, _)| key == &format!("reference:{}", row.id))
            {
                let observed_after = documents
                    .iter()
                    .find(|(k, _)| k == &format!("__content_observation:{}", row.id))
                    .and_then(|(_, v)| v.parse::<i64>().ok())
                    .unwrap_or(0);
                Self::observe_content_reference_tx(&mut tx, row, reference, observed_after).await?;
            }
        }
        tx.commit().await?;
        Ok(rows)
    }

    async fn previous_inbox_tx(tx: &mut sqlx::SqliteConnection, id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM messages m, json_each(m.document,'$.folder_ids') f WHERE m.id=? AND lower(ltrim(f.value,'\\'))='inbox')")
            .bind(id).fetch_one(&mut *tx).await?)
    }

    async fn upsert_message_tx(
        tx: &mut sqlx::SqliteConnection,
        mut message: Message,
    ) -> Result<Message> {
        sqlx::query("UPDATE messages SET revision=revision WHERE id=?")
            .bind(&message.id)
            .execute(&mut *tx)
            .await?;
        let mut previous = None;
        if let Some(row) =
            sqlx::query("SELECT revision,category_override,document FROM messages WHERE id=?")
                .bind(&message.id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let old: Message = serde_json::from_str(row.get("document"))?;
            if message.category.is_none() {
                message.category = old.category.clone();
                message.category_status = old.category_status.clone();
            }
            previous = Some(old);
            message.revision = row
                .get::<u32, _>("revision")
                .checked_add(1)
                .ok_or(Error::Invalid)?;
            if let Some(category) = row.get::<Option<String>, _>("category_override") {
                message.category = Some(category);
                message.category_status = CategoryStatus::Ready;
            }
        } else {
            message.revision = 1;
        }
        let pending:Vec<String>=sqlx::query_scalar("SELECT document FROM operations WHERE json_extract(document,'$.target_id')=? AND json_extract(document,'$.status')='pending' ORDER BY json_extract(document,'$.revision')").bind(&message.id).fetch_all(&mut *tx).await?;
        for raw in pending {
            let op: Operation = serde_json::from_str(&raw)?;
            match op.change {
                Change::Read { value } => message.unread = !value,
                Change::Star { value } => message.starred = value,
                _ => {}
            }
        }
        if let Some(old) = previous {
            let revision = message.revision;
            message.revision = old.revision;
            if message == old {
                return Ok(old);
            }
            message.revision = revision;
        }
        sqlx::query("INSERT INTO messages(id,revision,document) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document=excluded.document").bind(&message.id).bind(message.revision).bind(serde_json::to_string(&message)?).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'message_changed',?)")
            .bind(message.revision)
            .bind(&message.id)
            .execute(&mut *tx)
            .await?;
        Ok(message)
    }
    pub async fn message(&self, a: &AuthorizedAccount, id: &str) -> Result<Message> {
        let db = self.db(a).await?;
        let raw: Option<String> = sqlx::query_scalar("SELECT document FROM messages WHERE id=?")
            .bind(id)
            .fetch_optional(&db)
            .await?;
        Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
    }
    pub async fn catalogue(&self, a: &AuthorizedAccount) -> Result<Vec<Message>> {
        let db = self.db(a).await?;
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT document FROM messages ORDER BY id LIMIT 200")
                .fetch_all(&db)
                .await?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub async fn operation(&self, a: &AuthorizedAccount, op: Operation) -> Result<Operation> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let result = Self::operation_tx(&mut tx, a, op).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub(super) async fn operation_tx(
        tx: &mut sqlx::SqliteConnection,
        a: &AuthorizedAccount,
        mut op: Operation,
    ) -> Result<Operation> {
        if op.account_id != a.id || op.id.is_empty() {
            return Err(Error::Invalid);
        }
        op.status = OperationStatus::Pending;
        op.revision = 0;
        let request = serde_json::to_string(&op)?;
        sqlx::query("UPDATE operations SET request=request WHERE id=?")
            .bind(&op.id)
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query("SELECT request,document FROM operations WHERE id=?")
            .bind(&op.id)
            .fetch_optional(&mut *tx)
            .await?
        {
            if row.get::<String, _>("request") != request {
                return Err(Error::Conflict);
            }
            return Ok(serde_json::from_str(row.get("document"))?);
        }
        let raw: Option<String> =
            sqlx::query_scalar("SELECT document FROM messages WHERE id=? AND revision=?")
                .bind(&op.target_id)
                .bind(op.expected_revision)
                .fetch_optional(&mut *tx)
                .await?;
        let mut m: Message = serde_json::from_str(&raw.ok_or(Error::Conflict)?)?;
        match &op.change {
            Change::Read { value } => m.unread = !*value,
            Change::Star { value } => m.starred = *value,
            Change::Category { value } => {
                if ![
                    "people",
                    "transactions",
                    "notifications",
                    "newsletters",
                    "other",
                ]
                .contains(&value.as_str())
                {
                    return Err(Error::Invalid);
                }
                m.category = Some(value.clone());
                m.category_status = CategoryStatus::Ready;
                sqlx::query("UPDATE messages SET category_override=? WHERE id=?")
                    .bind(value)
                    .bind(&m.id)
                    .execute(&mut *tx)
                    .await?;
            }
            Change::Snooze { until } => {
                let until = DateTime::parse_from_rfc3339(until)
                    .map_err(|_| Error::Invalid)?
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Millis, true);
                sqlx::query("INSERT INTO snoozes(thread_id,until,arrival_boundary) VALUES(?,?,?) ON CONFLICT(thread_id) DO UPDATE SET until=excluded.until,arrival_boundary=excluded.arrival_boundary").bind(&m.thread_id).bind(until).bind(now()).execute(&mut *tx).await?;
            }
            // Provider adapter applies its discovered folder semantics. No invented folder IDs.
            Change::Archive | Change::Trash | Change::Spam | Change::Move { .. } => {}
        }
        m.revision = m.revision.checked_add(1).ok_or(Error::Invalid)?;
        op.revision = m.revision;
        if matches!(op.change, Change::Category { .. } | Change::Snooze { .. }) {
            op.status = OperationStatus::Applied
        }
        sqlx::query("UPDATE messages SET revision=?,document=? WHERE id=?")
            .bind(m.revision)
            .bind(serde_json::to_string(&m)?)
            .bind(&m.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO operations(id,request,document) VALUES(?,?,?)")
            .bind(&op.id)
            .bind(request)
            .bind(serde_json::to_string(&op)?)
            .execute(&mut *tx)
            .await?;
        if op.status == OperationStatus::Pending {
            sqlx::query("INSERT INTO outbox(kind,target_id) VALUES('mutation',?)")
                .bind(&op.id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,'operation_changed',?)")
            .bind(op.revision)
            .bind(&op.target_id)
            .execute(&mut *tx)
            .await?;
        Ok(op)
    }
}
