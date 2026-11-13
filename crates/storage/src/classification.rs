use super::*;

// Shared by every local query before LIMIT/keyset pagination. Overrides are
// materialized transactionally, so cached/provider projections use the same facts.
pub(crate) const CLASSIFICATION_SQL: &str = "
 AND (json_extract(cf.filter,'$.inbox_kind') IS NULL OR COALESCE(json_extract(m.document,'$.classification.kind'),'personal')=json_extract(cf.filter,'$.inbox_kind'))
 AND (json_extract(cf.filter,'$.email_type') IS NULL OR EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.types') t WHERE t.value=json_extract(cf.filter,'$.email_type')))
 AND (COALESCE(json_extract(cf.filter,'$.attention'),0)=0 OR (julianday(json_extract(m.document,'$.received_at')) BETWEEN julianday('now','-30 days') AND julianday('now') AND (json_extract(m.document,'$.classification.attention_override')=1 OR (json_extract(m.document,'$.classification.attention_override') IS NULL AND EXISTS(SELECT 1 FROM json_each(m.document,'$.classification.attention') a WHERE json_extract(a.value,'$.expires_at') IS NULL OR json_extract(a.value,'$.expires_at')>strftime('%Y-%m-%dT%H:%M:%fZ','now'))))))";
fn wire<T: serde::Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_value(v)?
        .as_str()
        .ok_or(Error::Invalid)?
        .into())
}
pub(crate) fn legacy_classification(category: &str) -> Classification {
    Classification {
        kind: if matches!(category, "people" | "other") {
            InboxKind::Personal
        } else {
            InboxKind::Automated
        },
        status: CategoryStatus::Ready,
        ..Default::default()
    }
}
impl Store {
    pub(crate) async fn apply_classification_tx(
        tx: &mut sqlx::SqliteConnection,
        m: &mut Message,
        automatic: Option<&Classification>,
    ) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO message_classifications(message_id) VALUES(?)")
            .bind(&m.id)
            .execute(&mut *tx)
            .await?;
        if let Some(c) = automatic {
            sqlx::query("UPDATE message_classifications SET automatic=? WHERE message_id=?")
                .bind(serde_json::to_string(c)?)
                .bind(&m.id)
                .execute(&mut *tx)
                .await?;
        }
        let row=sqlx::query("SELECT c.automatic,c.kind_override,c.added_types,c.removed_types,c.attention_override,m.category_override FROM message_classifications c LEFT JOIN messages m ON m.id=c.message_id WHERE c.message_id=?").bind(&m.id).fetch_one(&mut *tx).await?;
        let raw: Option<String> = row.get("automatic");
        let mut c: Classification = raw
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .or_else(|| m.classification.clone())
            .unwrap_or_default();
        if raw.is_none() && c.source != ClassificationSource::Automatic {
            c.kind = InboxKind::Personal;
        }
        let sender: Option<String> = sqlx::query_scalar(
            "SELECT kind FROM classification_senders WHERE sender=lower(trim(?))",
        )
        .bind(&m.sender.email)
        .fetch_optional(&mut *tx)
        .await?;
        let manual: Option<String> = row.get("kind_override");
        c.source = ClassificationSource::Automatic;
        if let Some(value) = manual.as_ref().or(sender.as_ref()) {
            c.kind = serde_json::from_value(serde_json::Value::String(value.clone()))?;
            c.source = if manual.is_some() {
                ClassificationSource::Message
            } else {
                ClassificationSource::Sender
            };
        }
        let add: Vec<DetectedType> = serde_json::from_str(row.get("added_types"))?;
        let remove: Vec<DetectedType> = serde_json::from_str(row.get("removed_types"))?;
        for t in add {
            if !c.types.contains(&t) {
                c.types.push(t);
            }
        }
        c.types.retain(|t| !remove.contains(t));
        c.attention_override = row.get("attention_override");
        m.category_status = c.status.clone();
        if c.status == CategoryStatus::Ready {
            m.category = Some(c.legacy_category().into());
            m.category_status = CategoryStatus::Ready;
        }
        if let Some(legacy) = row.get::<Option<String>, _>("category_override") {
            m.category = Some(legacy);
            m.category_status = CategoryStatus::Ready;
        }
        m.classification = Some(c);
        Ok(())
    }
    pub(crate) async fn classification_change_tx(
        tx: &mut sqlx::SqliteConnection,
        m: &mut Message,
        change: &Change,
    ) -> Result<()> {
        Self::apply_classification_tx(tx, m, None).await?;
        match change {
            Change::InboxKind { value, sender } => {
                let value = value.as_ref().map(wire).transpose()?;
                if *sender {
                    let email = crate::rules::normalize_rule_sender(&m.sender.email)?;
                    if let Some(v) = &value {
                        sqlx::query("INSERT INTO classification_senders(sender,kind) VALUES(?,?) ON CONFLICT(sender) DO UPDATE SET kind=excluded.kind").bind(&email).bind(v).execute(&mut *tx).await?;
                    } else {
                        sqlx::query("DELETE FROM classification_senders WHERE sender=?")
                            .bind(&email)
                            .execute(&mut *tx)
                            .await?;
                    }
                    // Apply the sender choice now; explicit corrections on other messages still win.
                    if value.is_some() {
                        sqlx::query("UPDATE message_classifications SET kind_override=NULL WHERE message_id=?").bind(&m.id).execute(&mut *tx).await?;
                    }
                    let affected = "lower(trim(json_extract(document,'$.sender.email')))=? AND id<>? AND NOT EXISTS(SELECT 1 FROM message_classifications c WHERE c.message_id=messages.id AND c.kind_override IS NOT NULL)";
                    sqlx::query(&format!("INSERT INTO events(revision,kind,target_id) SELECT revision+1,'message_changed',id FROM messages WHERE {affected}"))
                        .bind(&email).bind(&m.id).execute(&mut *tx).await?;
                    sqlx::query(&format!("UPDATE messages SET revision=revision+1,document=json_set(document,'$.revision',revision+1,'$.classification.kind',COALESCE(?,(SELECT json_extract(automatic,'$.kind') FROM message_classifications c WHERE c.message_id=messages.id),'personal'),'$.classification.source',CASE WHEN ? IS NULL THEN 'automatic' ELSE 'sender' END) WHERE {affected}"))
                        .bind(&value).bind(&value).bind(&email).bind(&m.id).execute(&mut *tx).await?;
                    sqlx::query(&format!("UPDATE messages SET document=json_set(document,'$.category',CASE WHEN json_extract(document,'$.classification.kind')='personal' THEN 'people' WHEN EXISTS(SELECT 1 FROM json_each(document,'$.classification.types') t WHERE t.value IN ('newsletter','offer')) THEN 'newsletters' WHEN EXISTS(SELECT 1 FROM json_each(document,'$.classification.types') t WHERE t.value IN ('receipt','invoice','order','shipping','flight_booking')) THEN 'transactions' ELSE 'notifications' END) WHERE {affected} AND json_extract(document,'$.classification.status')='ready'"))
                        .bind(&email).bind(&m.id).execute(&mut *tx).await?;
                } else {
                    sqlx::query(
                        "UPDATE message_classifications SET kind_override=? WHERE message_id=?",
                    )
                    .bind(&value)
                    .bind(&m.id)
                    .execute(&mut *tx)
                    .await?;
                }
                if !sender || value.is_some() {
                    sqlx::query("UPDATE messages SET category_override=NULL WHERE id=?")
                        .bind(&m.id)
                        .execute(&mut *tx)
                        .await?;
                }
            }
            Change::DetectedType { value, present } => {
                let row=sqlx::query("SELECT added_types,removed_types FROM message_classifications WHERE message_id=?").bind(&m.id).fetch_one(&mut *tx).await?;
                let mut added: Vec<DetectedType> = serde_json::from_str(row.get("added_types"))?;
                let mut removed: Vec<DetectedType> =
                    serde_json::from_str(row.get("removed_types"))?;
                added.retain(|t| t != value);
                removed.retain(|t| t != value);
                if *present {
                    added.push(value.clone());
                } else {
                    removed.push(value.clone());
                }
                sqlx::query("UPDATE message_classifications SET added_types=?,removed_types=? WHERE message_id=?").bind(serde_json::to_string(&added)?).bind(serde_json::to_string(&removed)?).bind(&m.id).execute(&mut *tx).await?;
            }
            Change::Attention { value } => {
                sqlx::query(
                    "UPDATE message_classifications SET attention_override=? WHERE message_id=?",
                )
                .bind(value)
                .bind(&m.id)
                .execute(&mut *tx)
                .await?;
            }
            Change::Category { value } => {
                sqlx::query(
                    "UPDATE message_classifications SET kind_override=? WHERE message_id=?",
                )
                .bind(wire(&legacy_classification(value).kind)?)
                .bind(&m.id)
                .execute(&mut *tx)
                .await?;
            }
            _ => return Err(Error::Invalid),
        }
        Self::apply_classification_tx(tx, m, None).await
    }
    pub async fn requeue_classification_policy(
        &self,
        a: &AuthorizedAccount,
        revision: &str,
    ) -> Result<()> {
        if self
            .provider_document(a, "classification-policy")
            .await?
            .as_deref()
            == Some(revision)
        {
            return Ok(());
        }
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE message_classifications SET automatic=NULL WHERE automatic IS NOT NULL AND COALESCE(json_extract(automatic,'$.policy_revision'),'')<>?")
            .bind(revision).execute(&mut *tx).await?;
        sqlx::query("UPDATE push_jobs SET state='skipped' WHERE state IN ('pending','running') AND EXISTS(SELECT 1 FROM messages m WHERE m.id=push_jobs.message_id AND COALESCE(json_extract(m.document,'$.classification.policy_revision'),'')<>?)")
            .bind(revision).execute(&mut *tx).await?;
        let stale="json_extract(document,'$.classification.status') IN ('ready','failed') AND COALESCE(json_extract(document,'$.classification.policy_revision'),'')<>?";
        sqlx::query(&format!("INSERT INTO events(revision,kind,target_id) SELECT revision+1,'message_changed',id FROM messages WHERE {stale}"))
            .bind(revision).execute(&mut *tx).await?;
        sqlx::query(&format!("UPDATE messages SET revision=revision+1,document=json_set(document,'$.revision',revision+1,'$.classification.status','pending','$.classification.kind',CASE WHEN json_extract(document,'$.classification.source') IN ('message','sender') THEN json_extract(document,'$.classification.kind') ELSE 'personal' END) WHERE {stale}"))
            .bind(revision).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO provider_documents(key,document) VALUES('classification-policy',?) ON CONFLICT(key) DO UPDATE SET document=excluded.document")
            .bind(revision).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn request_classification(
        &self,
        a: &AuthorizedAccount,
        ids: &[String],
    ) -> Result<()> {
        if ids.len() > 200 {
            return Err(Error::Invalid);
        }
        let db = self.db(a).await?;
        for id in ids {
            sqlx::query("INSERT INTO message_classifications(message_id,requested) SELECT id,1 FROM messages WHERE id=? ON CONFLICT(message_id) DO UPDATE SET requested=1").bind(id).execute(&db).await?;
        }
        Ok(())
    }
}
