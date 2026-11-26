//! Composable account transactions. Revision and mutation decisions are shared
//! across engines, including callers already inside a SQLite transaction.
use super::*;
use sqlx::{PgConnection, SqliteConnection};

enum Connection<'a> {
    Sqlite(&'a mut SqliteConnection),
    Postgres(&'a mut PgConnection),
}
pub(crate) struct MailboxTransaction<'a> {
    connection: Connection<'a>,
    account: String,
}

macro_rules! value {
    ($self:expr,$sql:expr,[$($arg:expr),*],$pg:expr,[$($p:expr),*],$method:ident) => {
        match &mut $self.connection {
            Connection::Sqlite(db)=>sqlx::query_scalar($sql)$(.bind($arg))*.$method(&mut **db).await?,
            Connection::Postgres(db)=>sqlx::query_scalar($pg)$(.bind($p))*.$method(&mut **db).await?,
        }
    };
}
macro_rules! tuples {
    ($self:expr,$sql:expr,[$($arg:expr),*],$pg:expr,[$($p:expr),*],$method:ident) => {
        match &mut $self.connection {
            Connection::Sqlite(db)=>sqlx::query_as($sql)$(.bind($arg))*.$method(&mut **db).await?,
            Connection::Postgres(db)=>sqlx::query_as($pg)$(.bind($p))*.$method(&mut **db).await?,
        }
    };
}
macro_rules! write {
    ($self:expr,$sql:expr,[$($arg:expr),*],$pg:expr,[$($p:expr),*]) => {
        match &mut $self.connection {
            Connection::Sqlite(db)=>sqlx::query($sql)$(.bind($arg))*.execute(&mut **db).await?.rows_affected(),
            Connection::Postgres(db)=>sqlx::query($pg)$(.bind($p))*.execute(&mut **db).await?.rows_affected(),
        }
    };
}

impl<'a> MailboxTransaction<'a> {
    pub(crate) fn sqlite(connection: &'a mut SqliteConnection, account: &str) -> Self {
        Self {
            connection: Connection::Sqlite(connection),
            account: account.into(),
        }
    }
    fn postgres(connection: &'a mut PgConnection, account: &str) -> Self {
        Self {
            connection: Connection::Postgres(connection),
            account: account.into(),
        }
    }
    async fn event(&mut self, revision: u32, kind: &str, target: &str) -> Result<()> {
        write!(self,"INSERT INTO events(revision,kind,target_id) VALUES(?,?,?)",[i64::from(revision),kind,target],"WITH next AS (UPDATE account_locks SET event_cursor=event_cursor+1 WHERE account_id=$1 RETURNING event_cursor) INSERT INTO events(account_id,cursor,revision,kind,target_id) SELECT $1,event_cursor,$2,$3,$4 FROM next",[&self.account,i64::from(revision),kind,target]);
        Ok(())
    }
    pub(crate) async fn message(&mut self, id: &str) -> Result<Message> {
        let raw: Option<String> = value!(
            self,
            "SELECT document FROM messages WHERE id=?",
            [id],
            "SELECT document FROM messages WHERE account_id=$1 AND id=$2",
            [&self.account, id],
            fetch_optional
        );
        Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
    }
    async fn store_message(&mut self, message: &Message) -> Result<()> {
        if message.account_id != self.account
            || message.received_date.as_deref().is_some_and(|day| {
                chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                    .map_or(true, |date| date.format("%Y-%m-%d").to_string() != day)
            })
        {
            return Err(Error::Invalid);
        }
        let raw = serde_json::to_string(message)?;
        let sender_key = crate::sender_key::normalize(&message.sender.email);
        write!(self,"INSERT INTO messages(id,revision,document,sender_key) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document=excluded.document,sender_key=excluded.sender_key",[&message.id,i64::from(message.revision),&raw,&sender_key],"INSERT INTO messages(account_id,id,revision,document,sender_key) VALUES($1,$2,$3,$4,$5) ON CONFLICT(account_id,id) DO UPDATE SET revision=excluded.revision,document=excluded.document,sender_key=excluded.sender_key",[&self.account,&message.id,i64::from(message.revision),&raw,&sender_key]);
        Ok(())
    }
    pub(crate) async fn upsert_message(&mut self, mut message: Message) -> Result<Message> {
        if message.account_id != self.account {
            return Err(Error::Invalid);
        }
        if message.classification.is_none() && message.category_status == CategoryStatus::Ready {
            message.classification = message
                .category
                .as_deref()
                .map(crate::classification::legacy_classification);
        }
        write!(
            self,
            "UPDATE messages SET revision=revision WHERE id=?",
            [&message.id],
            "UPDATE messages SET revision=revision WHERE account_id=$1 AND id=$2",
            [&self.account, &message.id]
        );
        let record:Option<(i64,Option<String>,String)>=tuples!(self,"SELECT revision,category_override,document FROM messages WHERE id=?",[&message.id],"SELECT revision,category_override,document FROM messages WHERE account_id=$1 AND id=$2",[&self.account,&message.id],fetch_optional);
        let mut previous = None;
        if let Some((revision, category_override, raw)) = record {
            let old: Message = serde_json::from_str(&raw)?;
            if message.category.is_none() {
                message.category = old.category.clone();
                message.category_status = old.category_status.clone();
            }
            message.classification = old.classification.clone();
            previous = Some(old);
            message.revision = u32::try_from(revision)
                .map_err(|_| Error::Invalid)?
                .checked_add(1)
                .ok_or(Error::Invalid)?;
            if let Some(category) = category_override {
                message.category = Some(category);
                message.category_status = CategoryStatus::Ready;
            }
        } else {
            message.revision = 1;
        }
        self.apply_classification(&mut message, None).await?;
        let pending:Vec<String>=value!(self,"SELECT document FROM operations WHERE json_extract(document,'$.target_id')=? AND json_extract(document,'$.status')='pending' ORDER BY json_extract(document,'$.revision')",[&message.id],"SELECT document FROM operations WHERE account_id=$1 AND target_id=$2 AND status='pending' ORDER BY revision",[&self.account,&message.id],fetch_all);
        for raw in pending {
            let operation: Operation = serde_json::from_str(&raw)?;
            match operation.change {
                Change::Read { value } => message.unread = !value,
                Change::Star { value } => message.starred = value,
                _ => {}
            }
        }
        if let Some(previous) = previous {
            let revision = message.revision;
            message.revision = previous.revision;
            if message == previous {
                return Ok(previous);
            }
            message.revision = revision;
        }
        self.store_message(&message).await?;
        self.event(message.revision, "message_changed", &message.id)
            .await?;
        Ok(message)
    }
    pub(crate) async fn operation(&mut self, mut operation: Operation) -> Result<Operation> {
        if operation.account_id != self.account || operation.id.is_empty() {
            return Err(Error::Invalid);
        }
        operation.status = OperationStatus::Pending;
        operation.revision = 0;
        let request = serde_json::to_string(&operation)?;
        write!(
            self,
            "UPDATE operations SET request=request WHERE id=?",
            [&operation.id],
            "UPDATE operations SET request=request WHERE account_id=$1 AND id=$2",
            [&self.account, &operation.id]
        );
        let existing: Option<(String, String)> = tuples!(
            self,
            "SELECT request,document FROM operations WHERE id=?",
            [&operation.id],
            "SELECT request,document FROM operations WHERE account_id=$1 AND id=$2",
            [&self.account, &operation.id],
            fetch_optional
        );
        if let Some((previous, raw)) = existing {
            if previous != request {
                return Err(Error::Conflict);
            }
            return Ok(serde_json::from_str(&raw)?);
        }
        let raw: Option<String> = value!(
            self,
            "SELECT document FROM messages WHERE id=? AND revision=?",
            [&operation.target_id, i64::from(operation.expected_revision)],
            "SELECT document FROM messages WHERE account_id=$1 AND id=$2 AND revision=$3",
            [
                &self.account,
                &operation.target_id,
                i64::from(operation.expected_revision)
            ],
            fetch_optional
        );
        let mut message: Message = serde_json::from_str(&raw.ok_or(Error::Conflict)?)?;
        match &operation.change {
            Change::Read { value } => message.unread = !*value,
            Change::Star { value } => message.starred = *value,
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
                message.category = Some(value.clone());
                message.category_status = CategoryStatus::Ready;
                write!(
                    self,
                    "UPDATE messages SET category_override=? WHERE id=?",
                    [value, &message.id],
                    "UPDATE messages SET category_override=$1 WHERE account_id=$2 AND id=$3",
                    [value, &self.account, &message.id]
                );
            }
            Change::InboxKind { .. } | Change::DetectedType { .. } | Change::Attention { .. } => {
                self.classification_change(&mut message, &operation.change)
                    .await?;
            }
            Change::Snooze { until } => {
                let until = DateTime::parse_from_rfc3339(until)
                    .map_err(|_| Error::Invalid)?
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(SecondsFormat::Millis, true);
                let boundary = now();
                write!(self,"INSERT INTO snoozes(thread_id,until,arrival_boundary) VALUES(?,?,?) ON CONFLICT(thread_id) DO UPDATE SET until=excluded.until,arrival_boundary=excluded.arrival_boundary",[&message.thread_id,&until,&boundary],"INSERT INTO snoozes(account_id,thread_id,until,arrival_boundary) VALUES($1,$2,$3,$4) ON CONFLICT(account_id,thread_id) DO UPDATE SET until=excluded.until,arrival_boundary=excluded.arrival_boundary",[&self.account,&message.thread_id,&until,&boundary]);
                write!(self,"DELETE FROM provider_documents WHERE key GLOB 'snooze-restore:*' AND document=?",[&message.thread_id],"DELETE FROM provider_documents WHERE account_id=$1 AND key LIKE 'snooze-restore:%' AND document=$2",[&self.account,&message.thread_id]);
            }
            Change::Move { folder_id } if folder_id.eq_ignore_ascii_case("INBOX") => {
                let queued:bool=value!(self,"SELECT EXISTS(SELECT 1 FROM outbox q JOIN operations o ON o.id=q.target_id WHERE q.kind='mutation' AND q.state IN ('pending','running') AND json_extract(o.document,'$.target_id')=?)",[&message.id],"SELECT EXISTS(SELECT 1 FROM outbox q JOIN operations o ON o.account_id=q.account_id AND o.id=q.target_id WHERE q.account_id=$1 AND q.kind='mutation' AND q.state IN ('pending','running') AND o.target_id=$2)",[&self.account,&message.id],fetch_one);
                if !queued
                    && message
                        .folder_ids
                        .iter()
                        .any(|folder| folder.eq_ignore_ascii_case("INBOX"))
                {
                    write!(
                        self,
                        "DELETE FROM snoozes WHERE thread_id=?",
                        [&message.thread_id],
                        "DELETE FROM snoozes WHERE account_id=$1 AND thread_id=$2",
                        [&self.account, &message.thread_id]
                    );
                    operation.status = OperationStatus::Applied;
                }
                if operation.status == OperationStatus::Pending {
                    let key = format!("snooze-restore:{}", operation.id);
                    write!(self,"INSERT INTO provider_documents(key,document) SELECT ?,thread_id FROM snoozes WHERE thread_id=?",[&key,&message.thread_id],"INSERT INTO provider_documents(account_id,key,document) SELECT $1,$2,thread_id FROM snoozes WHERE account_id=$1 AND thread_id=$3",[&self.account,&key,&message.thread_id]);
                }
                write!(self,"INSERT INTO rule_inbox_observations(message_id) VALUES(?) ON CONFLICT DO NOTHING",[&message.id],"INSERT INTO rule_inbox_observations(account_id,message_id) VALUES($1,$2) ON CONFLICT DO NOTHING",[&self.account,&message.id]);
            }
            Change::Archive | Change::Trash | Change::Spam | Change::Move { .. } => {}
        }
        if matches!(operation.change, Change::Category { .. }) {
            self.classification_change(&mut message, &operation.change)
                .await?;
        }
        message.revision = message.revision.checked_add(1).ok_or(Error::Invalid)?;
        operation.revision = message.revision;
        if matches!(
            operation.change,
            Change::Category { .. }
                | Change::InboxKind { .. }
                | Change::DetectedType { .. }
                | Change::Attention { .. }
                | Change::Snooze { .. }
        ) {
            operation.status = OperationStatus::Applied;
        }
        self.store_message(&message).await?;
        let raw = serde_json::to_string(&operation)?;
        write!(
            self,
            "INSERT INTO operations(id,request,document) VALUES(?,?,?)",
            [&operation.id, &request, &raw],
            "INSERT INTO operations(account_id,id,request,document) VALUES($1,$2,$3,$4)",
            [&self.account, &operation.id, &request, &raw]
        );
        if operation.status == OperationStatus::Pending {
            write!(
                self,
                "INSERT INTO outbox(kind,target_id) VALUES('mutation',?)",
                [&operation.id],
                "INSERT INTO outbox(account_id,kind,target_id) VALUES($1,'mutation',$2)",
                [&self.account, &operation.id]
            );
        }
        self.event(
            operation.revision,
            "operation_changed",
            &operation.target_id,
        )
        .await?;
        Ok(operation)
    }

    async fn previous_rule_arrival(&mut self, id: &str) -> Result<bool> {
        Ok(value!(self,"SELECT EXISTS(SELECT 1 FROM messages m, json_each(m.document,'$.folder_ids') f WHERE m.id=? AND (lower(ltrim(f.value,'\\')) IN ('inbox','trash') OR f.value=(SELECT json_extract(document,'$.trash_folder') FROM provider_documents WHERE key='mailbox-settings') OR f.value IN (SELECT json_extract(folder.value,'$.name') FROM provider_documents p,json_each(p.document,'$.folders') folder,json_each(folder.value,'$.attributes') attr WHERE p.key='probe' AND lower(ltrim(attr.value,'\\'))='trash')))",[id],"SELECT EXISTS(SELECT 1 FROM messages m CROSS JOIN LATERAL unnest(m.folder_ids) f WHERE m.account_id=$1 AND m.id=$2 AND (lower(ltrim(f,'\\')) IN ('inbox','trash') OR f=(SELECT document::jsonb->>'trash_folder' FROM provider_documents WHERE account_id=$1 AND key='mailbox-settings') OR f IN (SELECT folder->>'name' FROM provider_documents p CROSS JOIN LATERAL jsonb_array_elements(p.document::jsonb->'folders') folder CROSS JOIN LATERAL jsonb_array_elements_text(folder->'attributes') attr WHERE p.account_id=$1 AND p.key='probe' AND lower(ltrim(attr,'\\'))='trash')))",[&self.account,id],fetch_one))
    }
    async fn evaluate_rules(
        &mut self,
        message: &mut Message,
        already_seen: bool,
        rules: &[MailRule],
        connected: Option<&str>,
    ) -> Result<()> {
        if !message
            .folder_ids
            .iter()
            .any(|f| f.trim_start_matches('\\').eq_ignore_ascii_case("inbox"))
        {
            return Ok(());
        }
        let first=write!(self,"INSERT INTO rule_inbox_observations(message_id) VALUES(?) ON CONFLICT DO NOTHING",[&message.id],"INSERT INTO rule_inbox_observations(account_id,message_id) VALUES($1,$2) ON CONFLICT DO NOTHING",[&self.account,&message.id])==1;
        if !first
            || already_seen
            || !rules
                .iter()
                .any(|rule| crate::rules::rule_matches(rule, message))
        {
            return Ok(());
        }
        let mut excluded = vec![
            "trash".to_owned(),
            "sent".into(),
            "drafts".into(),
            "draft".into(),
        ];
        let documents:Vec<String>=value!(self,"SELECT document FROM provider_documents WHERE key IN ('mailbox-settings','probe')",[],"SELECT document FROM provider_documents WHERE account_id=$1 AND key IN ('mailbox-settings','probe')",[&self.account],fetch_all);
        for raw in documents {
            let doc: serde_json::Value = serde_json::from_str(&raw)?;
            for field in ["trash_folder", "sent_folder"] {
                if let Some(folder) = doc[field].as_str() {
                    excluded.push(folder.to_lowercase());
                }
            }
            if let Some(folders) = doc["folders"].as_array() {
                for folder in folders {
                    if folder["attributes"].as_array().is_some_and(|attrs| {
                        attrs.iter().any(|attr| {
                            attr.as_str().is_some_and(|s| {
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
        if connected
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|at| {
                DateTime::parse_from_rfc3339(&message.received_at)
                    .map_or(true, |received| received <= at)
            })
        {
            return Ok(());
        }
        for rule in rules {
            if !crate::rules::rule_matches(rule, message) {
                continue;
            }
            let conflict:bool=value!(self,"SELECT EXISTS(SELECT 1 FROM operations WHERE json_extract(document,'$.target_id')=? AND json_extract(document,'$.status') IN ('pending','failed') AND json_extract(document,'$.change.kind') IN ('archive','trash','spam','move') AND id NOT IN (SELECT operation_id FROM rule_executions WHERE state='cancelled' AND operation_id IS NOT NULL))",[&message.id],"SELECT EXISTS(SELECT 1 FROM operations WHERE account_id=$1 AND target_id=$2 AND status IN ('pending','failed') AND change_kind IN ('archive','trash','spam','move') AND id NOT IN (SELECT operation_id FROM rule_executions WHERE account_id=$1 AND state='cancelled' AND operation_id IS NOT NULL))",[&self.account,&message.id],fetch_one);
            let id = format!("rule:{}:{}", rule.id, message.id);
            let operation_id = (!conflict).then_some(&id);
            let state = if conflict { "conflict" } else { "queued" };
            let inserted=write!(self,"INSERT INTO rule_executions(rule_id,message_id,operation_id,state) VALUES(?,?,?,?) ON CONFLICT DO NOTHING",[&rule.id,&message.id,operation_id,state],"INSERT INTO rule_executions(account_id,rule_id,message_id,operation_id,state) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",[&self.account,&rule.id,&message.id,operation_id,state]);
            if inserted == 0 || conflict {
                continue;
            }
            let operation = self
                .operation(Operation {
                    id,
                    account_id: self.account.clone(),
                    target_id: message.id.clone(),
                    expected_revision: message.revision,
                    change: Change::Trash,
                    status: OperationStatus::Pending,
                    revision: 0,
                })
                .await?;
            message.revision = operation.revision;
        }
        Ok(())
    }
    async fn observe_reference(
        &mut self,
        message: &Message,
        reference: &str,
        observed_after: i64,
    ) -> Result<()> {
        if !message.provider_id.starts_with("gmail:") {
            return Ok(());
        }
        let key = format!("content:references:{}", message.id);
        let raw: Option<String> = value!(
            self,
            "SELECT document FROM provider_documents WHERE key=?",
            [&key],
            "SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2",
            [&self.account, &key],
            fetch_optional
        );
        let Some(raw) = raw else {
            return Ok(());
        };
        let mut state: ContentReferences = serde_json::from_str(&raw)?;
        let changed_source = state.source != message.provider_id;
        if !changed_source
            && ((state.blocked.contains(&reference.to_owned())
                && observed_after <= state.blocked_at)
                || (state.blocked.is_empty() && observed_after == 0))
        {
            return Ok(());
        }
        state.token = Uuid::new_v4().to_string();
        state.source = message.provider_id.clone();
        if changed_source {
            state.blocked.clear();
        } else {
            state.blocked.retain(|r| r != reference);
        }
        let raw = serde_json::to_string(&state)?;
        write!(
            self,
            "UPDATE provider_documents SET document=? WHERE key=?",
            [&raw, &key],
            "UPDATE provider_documents SET document=$1 WHERE account_id=$2 AND key=$3",
            [&raw, &self.account, &key]
        );
        write!(self,"UPDATE search_documents SET body_state='pending',retry_at=0 WHERE message_id=? AND source=? AND body_state='reference_unavailable'",[&message.id,&message.provider_id],"UPDATE search_documents SET body_state='pending',retry_at=0 WHERE account_id=$1 AND message_id=$2 AND source=$3 AND body_state='reference_unavailable'",[&self.account,&message.id,&message.provider_id]);
        Ok(())
    }
    async fn ingest(
        &mut self,
        messages: Vec<Message>,
        documents: &[(String, String)],
        rules: &[MailRule],
        connected: Option<&str>,
    ) -> Result<Vec<Message>> {
        if messages.len() > 200
            || messages.iter().any(|m| m.account_id != self.account)
            || documents.len() > 1000
            || documents
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
        {
            return Err(Error::Invalid);
        }
        let mut rows = Vec::with_capacity(messages.len());
        let mut known = Vec::with_capacity(messages.len());
        for message in messages {
            known.push(self.previous_rule_arrival(&message.id).await?);
            rows.push(self.upsert_message(message).await?);
        }
        for (key, document) in documents {
            if key.starts_with("__content_observation:") {
                continue;
            }
            write!(self,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE document<>excluded.document",[key,document],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document WHERE provider_documents.document<>excluded.document",[&self.account,key,document]);
        }
        for (row, known) in rows.iter_mut().zip(known) {
            self.evaluate_rules(row, known, rules, connected).await?;
            if let Some((_, reference)) = documents
                .iter()
                .rev()
                .find(|(key, _)| key == &format!("reference:{}", row.id))
            {
                let observed_after = documents
                    .iter()
                    .find(|(key, _)| key == &format!("__content_observation:{}", row.id))
                    .and_then(|(_, value)| value.parse::<i64>().ok())
                    .unwrap_or(0);
                self.observe_reference(row, reference, observed_after)
                    .await?;
            }
        }
        Ok(rows)
    }
}

// A transaction scope can call several shared algorithms before committing.
// PostgreSQL ownership admission and account write serialization are centralized.
macro_rules! scoped {
    ($self:expr,$scope:ident,$write:expr,$body:block) => {
        match &$self.backend {
            Backend::Sqlite(db) => {
                let mut tx = if $write {
                    db.begin_with("BEGIN IMMEDIATE").await?
                } else {
                    db.begin().await?
                };
                let mut $scope = MailboxTransaction::sqlite(&mut tx, &$self.account);
                let result = ($body)?;
                drop($scope);
                tx.commit().await?;
                Ok::<_, Error>(result)
            }
            Backend::Postgres(db) => {
                let mut tx = $self.begin_postgres(db, $write).await?;
                let mut $scope = MailboxTransaction::postgres(&mut tx, &$self.account);
                let result = ($body)?;
                drop($scope);
                tx.commit().await?;
                Ok::<_, Error>(result)
            }
        }
    };
}
impl AccountRepository {
    pub(crate) async fn ingest_messages(
        &self,
        messages: Vec<Message>,
        documents: &[(String, String)],
        rules: &[MailRule],
        connected: Option<&str>,
    ) -> Result<Vec<Message>> {
        scoped!(self, scope, true, {
            scope.ingest(messages, documents, rules, connected).await
        })
    }
    pub(crate) async fn message(&self, id: &str) -> Result<Message> {
        scoped!(self, scope, false, { scope.message(id).await })
    }
    pub(crate) async fn operation(&self, operation: Operation) -> Result<Operation> {
        scoped!(self, scope, true, { scope.operation(operation).await })
    }
    pub(crate) async fn operation_status(&self, id: &str) -> Result<Operation> {
        transaction!(self, tx, local, false, {
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM operations WHERE id=?",
                [id],
                "SELECT document FROM operations WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
        })
    }
    pub(crate) async fn catalogue(&self) -> Result<Vec<Message>> {
        transaction!(self, tx, local, false, {
            let rows: Vec<String> = scalar!(
                local,
                tx,
                "SELECT document FROM messages ORDER BY id LIMIT 200",
                [],
                "SELECT document FROM messages WHERE account_id=$1 ORDER BY id LIMIT 200",
                [&self.account],
                fetch_all
            );
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn visible_message(&self, mut message: Message) -> Result<Message> {
        if message.account_id != self.account {
            return Err(Error::Invalid);
        }
        transaction!(self, tx, local, false, {
            let pending:Option<String>=scalar!(local,tx,"SELECT json_extract(document,'$.change.kind') FROM operations WHERE json_extract(document,'$.target_id')=? AND json_extract(document,'$.status')='pending' AND json_extract(document,'$.change.kind') IN ('archive','spam','trash') ORDER BY json_extract(document,'$.change.kind')='trash' DESC LIMIT 1",[&message.id],"SELECT change_kind FROM operations WHERE account_id=$1 AND target_id=$2 AND status='pending' AND change_kind IN ('archive','spam','trash') ORDER BY (change_kind='trash') DESC LIMIT 1",[&self.account,&message.id],fetch_optional);
            if pending.as_deref() == Some("trash") {
                message.folder_ids = vec!["Trash".into()];
            } else if pending.is_some() {
                message
                    .folder_ids
                    .retain(|f| !f.eq_ignore_ascii_case("INBOX") && f != "\\Inbox");
            }
            Ok(message)
        })
    }
}

#[path = "mailbox_cache.rs"]
mod cache;
#[path = "mailbox_cleanup.rs"]
mod cleanup;
#[path = "mailbox_graph.rs"]
mod graph;
#[path = "mailbox_models.rs"]
mod models;
#[path = "mailbox_mutations.rs"]
mod mutations;
#[path = "mailbox_notifications.rs"]
mod notifications;
#[path = "mailbox_references.rs"]
mod references;
#[path = "mailbox_scans.rs"]
mod scans;

#[path = "mailbox_search.rs"]
mod search;

#[path = "mailbox_search_delivery.rs"]
mod search_delivery;
#[path = "mailbox_semantic.rs"]
mod semantic;
impl AccountRepository {
    /// Materialize a newly connected account's scoped state before workers run.
    pub(crate) async fn initialize_account(&self) -> Result<()> {
        scoped!(self, scope, true, {
            let _ = &mut scope;
            Ok::<(), Error>(())
        })
    }
    pub(crate) async fn removal_busy(&self) -> Result<bool> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM submissions WHERE state IN ('scheduled','claimed','submitting','delivery_unknown') OR (state='accepted' AND filing_status IN ('pending','failed'))) OR EXISTS(SELECT 1 FROM outbox WHERE kind='mutation' AND state='running')",[],"SELECT EXISTS(SELECT 1 FROM submissions WHERE account_id=$1 AND (state IN ('scheduled','claimed','submitting','delivery_unknown') OR (state='accepted' AND filing_status IN ('pending','failed')))) OR EXISTS(SELECT 1 FROM outbox WHERE account_id=$1 AND kind='mutation' AND state='running')",[&self.account],fetch_one))
        })
    }
    pub(crate) async fn close_removed_account(&self) {
        // A PostgreSQL pool is shared across accounts and must remain open.
        if let Backend::Sqlite(db) = &self.backend {
            db.close().await;
        }
    }
}

#[path = "mailbox_classification.rs"]
mod classification;
