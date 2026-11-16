use super::*;
use crate::classification::{legacy_classification, wire};

impl MailboxTransaction<'_> {
    pub(super) async fn apply_classification(
        &mut self,
        m: &mut Message,
        automatic: Option<&Classification>,
    ) -> Result<()> {
        write!(self,"INSERT INTO message_classifications(message_id) VALUES(?) ON CONFLICT DO NOTHING",[&m.id],"INSERT INTO message_classifications(account_id,message_id) VALUES($1,$2) ON CONFLICT DO NOTHING",[&self.account,&m.id]);
        if let Some(c) = automatic {
            let raw = serde_json::to_string(c)?;
            write!(self,"UPDATE message_classifications SET automatic=? WHERE message_id=?",[&raw,&m.id],"UPDATE message_classifications SET automatic=$1 WHERE account_id=$2 AND message_id=$3",[&raw,&self.account,&m.id]);
        }
        let (raw,manual,added,removed,attention,legacy):(Option<String>,Option<String>,String,String,Option<bool>,Option<String>) = tuples!(self,"SELECT c.automatic,c.kind_override,c.added_types,c.removed_types,c.attention_override,m.category_override FROM message_classifications c LEFT JOIN messages m ON m.id=c.message_id WHERE c.message_id=?",[&m.id],"SELECT c.automatic,c.kind_override,c.added_types,c.removed_types,c.attention_override,m.category_override FROM message_classifications c LEFT JOIN messages m ON m.account_id=c.account_id AND m.id=c.message_id WHERE c.account_id=$1 AND c.message_id=$2",[&self.account,&m.id],fetch_one);
        let mut c: Classification = raw
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .or_else(|| m.classification.clone())
            .unwrap_or_default();
        if raw.is_none() && c.source != ClassificationSource::Automatic {
            c.kind = InboxKind::Personal;
        }
        let sender:Option<String> = value!(self,"SELECT kind FROM classification_senders WHERE sender=lower(trim(?))",[&m.sender.email],"SELECT kind FROM classification_senders WHERE account_id=$1 AND sender=lower(trim($2))",[&self.account,&m.sender.email],fetch_optional);
        c.source = ClassificationSource::Automatic;
        if let Some(value) = manual.as_ref().or(sender.as_ref()) {
            c.kind = serde_json::from_value(serde_json::Value::String(value.clone()))?;
            c.source = if manual.is_some() {
                ClassificationSource::Message
            } else {
                ClassificationSource::Sender
            };
        }
        let added: Vec<DetectedType> = serde_json::from_str(&added)?;
        let removed: Vec<DetectedType> = serde_json::from_str(&removed)?;
        for t in added {
            if !c.types.contains(&t) {
                c.types.push(t);
            }
        }
        c.types.retain(|t| !removed.contains(t));
        c.attention_override = attention;
        m.category_status = c.status.clone();
        if c.status == CategoryStatus::Ready {
            m.category = Some(c.legacy_category().into());
        }
        if let Some(legacy) = legacy {
            m.category = Some(legacy);
            m.category_status = CategoryStatus::Ready;
        }
        m.classification = Some(c);
        Ok(())
    }
    pub(super) async fn classification_change(
        &mut self,
        m: &mut Message,
        change: &Change,
    ) -> Result<()> {
        self.apply_classification(m, None).await?;
        match change {
            Change::InboxKind { value, sender } => {
                let value = value.as_ref().map(wire).transpose()?;
                if *sender {
                    let email = crate::rules::normalize_rule_sender(&m.sender.email)?;
                    if let Some(v) = &value {
                        write!(self,"INSERT INTO classification_senders(sender,kind) VALUES(?,?) ON CONFLICT(sender) DO UPDATE SET kind=excluded.kind",[&email,v],"INSERT INTO classification_senders(account_id,sender,kind) VALUES($1,$2,$3) ON CONFLICT(account_id,sender) DO UPDATE SET kind=excluded.kind",[&self.account,&email,v]);
                    } else {
                        write!(
                            self,
                            "DELETE FROM classification_senders WHERE sender=?",
                            [&email],
                            "DELETE FROM classification_senders WHERE account_id=$1 AND sender=$2",
                            [&self.account, &email]
                        );
                    }
                    if value.is_some() {
                        write!(self,"UPDATE message_classifications SET kind_override=NULL WHERE message_id=?",[&m.id],"UPDATE message_classifications SET kind_override=NULL WHERE account_id=$1 AND message_id=$2",[&self.account,&m.id]);
                    }
                    // Page within the same write transaction so a sender correction has bounded memory.
                    let mut after = String::new();
                    loop {
                        let rows:Vec<String> = value!(self,"SELECT document FROM messages WHERE lower(trim(json_extract(document,'$.sender.email')))=? AND id<>? AND id>? AND NOT EXISTS(SELECT 1 FROM message_classifications c WHERE c.message_id=messages.id AND c.kind_override IS NOT NULL) ORDER BY id LIMIT 200",[&email,&m.id,&after],"SELECT document FROM messages WHERE account_id=$1 AND lower(trim(document::jsonb#>>'{sender,email}'))=$2 AND id<>$3 AND id COLLATE \"C\">$4 AND NOT EXISTS(SELECT 1 FROM message_classifications c WHERE c.account_id=messages.account_id AND c.message_id=messages.id AND c.kind_override IS NOT NULL) ORDER BY id COLLATE \"C\" LIMIT 200",[&self.account,&email,&m.id,&after],fetch_all);
                        if rows.is_empty() {
                            break;
                        }
                        for raw in rows {
                            let mut other: Message = serde_json::from_str(&raw)?;
                            after = other.id.clone();
                            self.apply_classification(&mut other, None).await?;
                            other.revision = other.revision.checked_add(1).ok_or(Error::Invalid)?;
                            self.store_message(&other).await?;
                            self.event(other.revision, "message_changed", &other.id)
                                .await?;
                        }
                    }
                } else {
                    write!(self,"UPDATE message_classifications SET kind_override=? WHERE message_id=?",[&value,&m.id],"UPDATE message_classifications SET kind_override=$1 WHERE account_id=$2 AND message_id=$3",[&value,&self.account,&m.id]);
                }
                if !sender || value.is_some() {
                    write!(
                        self,
                        "UPDATE messages SET category_override=NULL WHERE id=?",
                        [&m.id],
                        "UPDATE messages SET category_override=NULL WHERE account_id=$1 AND id=$2",
                        [&self.account, &m.id]
                    );
                }
            }
            Change::DetectedType { value, present } => {
                let (added,removed):(String,String) = tuples!(self,"SELECT added_types,removed_types FROM message_classifications WHERE message_id=?",[&m.id],"SELECT added_types,removed_types FROM message_classifications WHERE account_id=$1 AND message_id=$2",[&self.account,&m.id],fetch_one);
                let mut added: Vec<DetectedType> = serde_json::from_str(&added)?;
                let mut removed: Vec<DetectedType> = serde_json::from_str(&removed)?;
                added.retain(|t| t != value);
                removed.retain(|t| t != value);
                if *present {
                    added.push(value.clone());
                } else {
                    removed.push(value.clone());
                }
                let added = serde_json::to_string(&added)?;
                let removed = serde_json::to_string(&removed)?;
                write!(self,"UPDATE message_classifications SET added_types=?,removed_types=? WHERE message_id=?",[&added,&removed,&m.id],"UPDATE message_classifications SET added_types=$1,removed_types=$2 WHERE account_id=$3 AND message_id=$4",[&added,&removed,&self.account,&m.id]);
            }
            Change::Attention { value } => {
                write!(self,"UPDATE message_classifications SET attention_override=? WHERE message_id=?",[value,&m.id],"UPDATE message_classifications SET attention_override=$1 WHERE account_id=$2 AND message_id=$3",[value,&self.account,&m.id]);
            }
            Change::Category { value } => {
                let kind = wire(&legacy_classification(value).kind)?;
                write!(self,"UPDATE message_classifications SET kind_override=? WHERE message_id=?",[&kind,&m.id],"UPDATE message_classifications SET kind_override=$1 WHERE account_id=$2 AND message_id=$3",[&kind,&self.account,&m.id]);
            }
            _ => return Err(Error::Invalid),
        }
        self.apply_classification(m, None).await
    }
    async fn requeue_classification_policy(&mut self, revision: &str) -> Result<()> {
        let current:Option<String> = value!(self,"SELECT document FROM provider_documents WHERE key='classification-policy'",[],"SELECT document FROM provider_documents WHERE account_id=$1 AND key='classification-policy'",[&self.account],fetch_optional);
        if current.as_deref() == Some(revision) {
            return Ok(());
        }
        write!(self,"UPDATE message_classifications SET automatic=NULL WHERE automatic IS NOT NULL AND COALESCE(json_extract(automatic,'$.policy_revision'),'')<>?",[revision],"UPDATE message_classifications SET automatic=NULL WHERE account_id=$1 AND automatic IS NOT NULL AND COALESCE(automatic::jsonb->>'policy_revision','')<>$2",[&self.account,revision]);
        write!(self,"UPDATE push_jobs SET state='skipped' WHERE state IN ('pending','running') AND EXISTS(SELECT 1 FROM messages m WHERE m.id=push_jobs.message_id AND COALESCE(json_extract(m.document,'$.classification.policy_revision'),'')<>?)",[revision],"UPDATE push_jobs p SET state='skipped' WHERE account_id=$1 AND state IN ('pending','running') AND EXISTS(SELECT 1 FROM messages m WHERE m.account_id=p.account_id AND m.id=p.message_id AND COALESCE(m.document::jsonb#>>'{classification,policy_revision}','')<>$2)",[&self.account,revision]);
        let mut after = String::new();
        loop {
            let rows:Vec<String> = value!(self,"SELECT document FROM messages WHERE id>? AND json_extract(document,'$.classification.status') IN ('ready','failed') AND COALESCE(json_extract(document,'$.classification.policy_revision'),'')<>? ORDER BY id LIMIT 200",[&after,revision],"SELECT document FROM messages WHERE account_id=$1 AND id COLLATE \"C\">$2 AND document::jsonb#>>'{classification,status}' IN ('ready','failed') AND COALESCE(document::jsonb#>>'{classification,policy_revision}','')<>$3 ORDER BY id COLLATE \"C\" LIMIT 200",[&self.account,&after,revision],fetch_all);
            if rows.is_empty() {
                break;
            }
            for raw in rows {
                let mut message: Message = serde_json::from_str(&raw)?;
                after = message.id.clone();
                if let Some(c) = message.classification.as_mut() {
                    c.status = CategoryStatus::Pending;
                    if !matches!(
                        c.source,
                        ClassificationSource::Message | ClassificationSource::Sender
                    ) {
                        c.kind = InboxKind::Personal;
                    }
                }
                message.revision = message.revision.checked_add(1).ok_or(Error::Invalid)?;
                self.store_message(&message).await?;
                self.event(message.revision, "message_changed", &message.id)
                    .await?;
            }
        }
        write!(self,"INSERT INTO provider_documents(key,document) VALUES('classification-policy',?) ON CONFLICT(key) DO UPDATE SET document=excluded.document",[revision],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,'classification-policy',$2) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&self.account,revision]);
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn requeue_classification_policy(&self, revision: &str) -> Result<()> {
        scoped!(self, scope, true, {
            scope.requeue_classification_policy(revision).await
        })
    }
    pub(crate) async fn request_classification(&self, ids: &[String]) -> Result<()> {
        scoped!(self, scope, true, {
            for id in ids {
                write!(scope,"INSERT INTO message_classifications(message_id,requested) SELECT id,1 FROM messages WHERE id=? ON CONFLICT(message_id) DO UPDATE SET requested=1",[id],"INSERT INTO message_classifications(account_id,message_id,requested) SELECT account_id,id,TRUE FROM messages WHERE account_id=$1 AND id=$2 ON CONFLICT(account_id,message_id) DO UPDATE SET requested=TRUE",[&scope.account,id]);
            }
            Ok::<_, Error>(())
        })
    }
}
