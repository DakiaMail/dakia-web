//! Read failures block exact Gmail references, not the stable Gmail identity.
use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct ContentReferences {
    pub token: String,
    pub source: String,
    pub blocked: Vec<String>,
    #[serde(default)]
    pub blocked_at: i64,
}
impl Store {
    pub async fn content_references(
        &self,
        a: &AuthorizedAccount,
        m: &Message,
    ) -> Result<ContentReferences> {
        if m.account_id != a.id || !m.provider_id.starts_with("gmail:") {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let initial = ContentReferences {
            token: Uuid::new_v4().to_string(),
            source: m.provider_id.clone(),
            blocked: vec![],
            blocked_at: 0,
        };
        let key = format!("content:references:{}", m.id);
        let mut tx = db.begin().await?;
        sqlx::query("INSERT INTO provider_documents(key,document) SELECT ?,? WHERE EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE json_extract(document,'$.source')<>json_extract(excluded.document,'$.source')")
            .bind(&key).bind(serde_json::to_string(&initial)?).bind(&m.id).bind(&m.provider_id).execute(&mut *tx).await?;
        let raw: String = sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=?")
            .bind(&key)
            .fetch_one(&mut *tx)
            .await?;
        let state: ContentReferences = serde_json::from_str(&raw)?;
        if state.source != m.provider_id {
            return Err(Error::Conflict);
        }
        tx.commit().await?;
        Ok(state)
    }
    pub async fn block_content_reference(
        &self,
        a: &AuthorizedAccount,
        m: &Message,
        reference: &str,
        token: &str,
    ) -> Result<bool> {
        if m.account_id != a.id {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let key = format!("content:references:{}", m.id);
        // Acquire the writer before reading the observation token.
        sqlx::query("UPDATE provider_documents SET document=document WHERE key=?")
            .bind(&key)
            .execute(&mut *tx)
            .await?;
        let raw:Option<String>=sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_extract(document,'$.source')=? AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=?)")
            .bind(&key).bind(token).bind(&m.provider_id).bind(&m.id).bind(&m.provider_id).fetch_optional(&mut *tx).await?;
        let Some(raw) = raw else { return Ok(false) };
        let mut state: ContentReferences = serde_json::from_str(&raw)?;
        if !state.blocked.iter().any(|r| r == reference) {
            if state.blocked.len() >= 257 {
                return Err(Error::Invalid);
            }
            state.blocked.push(reference.into());
            state.blocked_at = Utc::now().timestamp_micros();
            sqlx::query("UPDATE provider_documents SET document=? WHERE key=?")
                .bind(serde_json::to_string(&state)?)
                .bind(&key)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }
    /// Commit the gap only if no positive observation raced the failed reads.
    pub async fn mark_reference_gap(
        &self,
        a: &AuthorizedAccount,
        m: &Message,
        token: &str,
    ) -> Result<bool> {
        if m.account_id != a.id {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let changed=sqlx::query("UPDATE search_documents SET body_state='reference_unavailable',retry_at=CASE WHEN body_state='reference_unavailable' THEN retry_at ELSE 0 END,updated_at=? WHERE message_id=? AND source=? AND body_state IN ('pending','fetch_error','reference_unavailable') AND EXISTS(SELECT 1 FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_extract(document,'$.source')=? AND json_array_length(document,'$.blocked')>0) AND EXISTS(SELECT 1 FROM messages WHERE id=? AND json_extract(document,'$.provider_id')=?) AND EXISTS(SELECT 1 FROM provider_documents p,json_each((SELECT document FROM provider_documents WHERE key=?),'$.blocked') b WHERE p.key=? AND p.document=b.value)")
            .bind(Utc::now().timestamp()).bind(&m.id).bind(&m.provider_id).bind(format!("content:references:{}",m.id)).bind(token).bind(&m.provider_id).bind(&m.id).bind(&m.provider_id).bind(format!("content:references:{}",m.id)).bind(format!("reference:{}",m.id)).execute(&db).await?.rows_affected();
        Ok(changed > 0)
    }
    pub async fn promote_content_reference(
        &self,
        a: &AuthorizedAccount,
        m: &Message,
        reference: &str,
        token: &str,
    ) -> Result<()> {
        if m.account_id != a.id {
            return Err(Error::Invalid);
        }
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let key = format!("content:references:{}", m.id);
        sqlx::query("UPDATE provider_documents SET document=document WHERE key=?")
            .bind(&key)
            .execute(&mut *tx)
            .await?;
        let raw:Option<String>=sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_extract(document,'$.source')=?")
            .bind(&key).bind(token).bind(&m.provider_id).fetch_optional(&mut *tx).await?;
        if let Some(raw) = raw {
            let mut state: ContentReferences = serde_json::from_str(&raw)?;
            state.blocked.retain(|r| r != reference);
            state.token = Uuid::new_v4().to_string();
            sqlx::query("UPDATE provider_documents SET document=? WHERE key=?")
                .bind(reference)
                .bind(format!("reference:{}", m.id))
                .execute(&mut *tx)
                .await?;
            if state.blocked.is_empty() {
                sqlx::query("DELETE FROM provider_documents WHERE key=?")
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query("UPDATE provider_documents SET document=? WHERE key=?")
                    .bind(serde_json::to_string(&state)?)
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn forget_empty_content_references(
        &self,
        a: &AuthorizedAccount,
        id: &str,
        token: &str,
    ) -> Result<()> {
        let db = self.db(a).await?;
        sqlx::query("DELETE FROM provider_documents WHERE key=? AND json_extract(document,'$.token')=? AND json_array_length(document,'$.blocked')=0")
            .bind(format!("content:references:{id}")).bind(token).execute(&db).await?;
        Ok(())
    }
    pub(crate) async fn observe_content_reference_tx(
        tx: &mut sqlx::SqliteConnection,
        m: &Message,
        reference: &str,
        observed_after: i64,
    ) -> Result<()> {
        if !m.provider_id.starts_with("gmail:") {
            return Ok(());
        }
        let key = format!("content:references:{}", m.id);
        let raw: Option<String> =
            sqlx::query_scalar("SELECT document FROM provider_documents WHERE key=?")
                .bind(&key)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(raw) = raw else { return Ok(()) };
        let mut state: ContentReferences = serde_json::from_str(&raw)?;
        let changed_source = state.source != m.provider_id;
        if !changed_source
            && ((state.blocked.contains(&reference.to_owned())
                && observed_after <= state.blocked_at)
                || (state.blocked.is_empty() && observed_after == 0))
        {
            return Ok(());
        }

        state.token = Uuid::new_v4().to_string();
        state.source = m.provider_id.clone();
        if changed_source {
            state.blocked.clear()
        } else {
            state.blocked.retain(|r| r != reference)
        }
        sqlx::query("UPDATE provider_documents SET document=? WHERE key=?")
            .bind(serde_json::to_string(&state)?)
            .bind(key)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE search_documents SET body_state='pending',retry_at=0 WHERE message_id=? AND source=? AND body_state='reference_unavailable'")
            .bind(&m.id).bind(&m.provider_id).execute(tx).await?;
        Ok(())
    }
    /// At most one identity lookup/account/5 minutes and one/message/day.
    /// Persist admission before I/O so process death cannot cause a lookup storm.
    pub async fn claim_reference_recovery(&self, a: &AuthorizedAccount) -> Result<Option<Message>> {
        let _guard = self.gate.read().await;
        let db = self.db(a).await?;
        let mut tx = db.begin().await?;
        let now = Utc::now().timestamp();
        let admitted=sqlx::query("INSERT INTO provider_documents(key,document) VALUES('content:recovery_due',?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE CAST(document AS INTEGER)<=?")
            .bind((now+300).to_string()).bind(now).execute(&mut *tx).await?.rows_affected();
        if admitted == 0 {
            return Ok(None);
        }
        let raw:Option<String>=sqlx::query_scalar("SELECT m.document FROM search_documents d JOIN messages m ON m.id=d.message_id WHERE d.body_state='reference_unavailable' AND d.retry_at<=? AND d.source=json_extract(m.document,'$.provider_id') AND json_array_length(m.document,'$.folder_ids')>0 ORDER BY d.retry_at,d.message_id LIMIT 1")
            .bind(now).fetch_optional(&mut *tx).await?;
        let Some(raw) = raw else { return Ok(None) };
        let m: Message = serde_json::from_str(&raw)?;
        sqlx::query("UPDATE search_documents SET retry_at=? WHERE message_id=? AND source=? AND body_state='reference_unavailable'").bind(now+86400).bind(&m.id).bind(&m.provider_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(m))
    }
}
