//! Durable model work in the selected search database. Embeddings and checkpoint
//! advance together; no model call or cross-database transaction occurs here.
use super::*;
use chrono::Utc;
use uuid::Uuid;
const WINDOW: usize = 16 * 1024;
#[derive(Debug, Clone)]
pub struct EmbeddingTurn {
    pub account_id: String,
    pub message_id: String,
    pub model_revision: String,
    pub content_version: i64,
    pub lease_token: String,
    pub field: i64,
    pub byte_offset: i64,
    pub ordinal: i64,
    pub text: String,
    pub total_bytes: i64,
    pub source: String,
}
#[derive(Debug, Clone)]
pub struct EmbeddedPassage {
    pub text: String,
    pub byte_end: usize,
    pub next_offset: usize,
    pub vector: Option<Vec<f32>>,
}
#[derive(Debug, Default, Clone)]
pub struct EmbeddingCoverage {
    pub model_revision: Option<String>,
    pub model_available: bool,
    pub discovery_complete: bool,
    pub pending: i64,
    pub processed: i64,
    pub gaps: i64,
    pub retrying: i64,
}
fn revision_valid(revision: &str) -> bool {
    !revision.is_empty() && revision.len() <= 128 && !revision.contains('\0')
}
fn utf8_window(mut bytes: Vec<u8>) -> Result<String> {
    if let Err(error) = std::str::from_utf8(&bytes) {
        if error.error_len().is_some() {
            return Err(Error::Invalid);
        }
        bytes.truncate(error.valid_up_to());
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid)
}
impl SearchDatabase {
    pub async fn prepare_embeddings(
        &self,
        account: &str,
        revision: &str,
        available: bool,
    ) -> Result<()> {
        if !revision_valid(revision) {
            return Err(Error::Invalid);
        }
        transaction!(self, account, tx, local, {
            let state = if available { "ready" } else { "unavailable" };
            write!(local,tx,"INSERT INTO embedding_accounts(account_id,model_revision,model_state,pending) VALUES(?,?,?,(SELECT count(*) FROM embedding_jobs WHERE account_id=?)) ON CONFLICT(account_id) DO UPDATE SET model_revision=excluded.model_revision,model_state=excluded.model_state,sweep_cursor=CASE WHEN embedding_accounts.model_revision=excluded.model_revision THEN embedding_accounts.sweep_cursor ELSE '' END,sweep_done=CASE WHEN embedding_accounts.model_revision=excluded.model_revision THEN embedding_accounts.sweep_done ELSE 0 END",[account,revision,state,account],"INSERT INTO embedding_accounts(account_id,model_revision,model_state,pending) VALUES($1,$2,$3,(SELECT count(*) FROM embedding_jobs WHERE account_id=$1)) ON CONFLICT(account_id) DO UPDATE SET model_revision=excluded.model_revision,model_state=excluded.model_state,sweep_cursor=CASE WHEN embedding_accounts.model_revision=excluded.model_revision THEN embedding_accounts.sweep_cursor ELSE '' END,sweep_done=CASE WHEN embedding_accounts.model_revision=excluded.model_revision THEN embedding_accounts.sweep_done ELSE 0 END",[account,revision,state]);
            tx.commit().await?;
            Ok(())
        })
    }
    /// Sweep at most 64 identities, including pre-migration documents. New
    /// arrivals enqueue through triggers and do not wait behind this discovery.
    pub async fn discover_embeddings(&self, account: &str, revision: &str) -> Result<bool> {
        transaction!(self, account, tx, local, {
            let cursor:Option<String>=query!(local,tx,"SELECT sweep_cursor FROM embedding_accounts WHERE account_id=? AND model_revision=? AND sweep_done=0",[account,revision],"SELECT sweep_cursor FROM embedding_accounts WHERE account_id=$1 AND model_revision=$2 AND sweep_done=0",[account,revision],fetch_optional);
            let Some(cursor) = cursor else {
                return Ok(false);
            };
            let page:Vec<(String,i64,String,String)>=rows!(local,tx,"SELECT message_id,content_version,embedding_revision,state FROM documents WHERE account_id=? AND message_id>? AND state<>'deleted' ORDER BY message_id LIMIT 64",[account,&cursor],"SELECT message_id,content_version,embedding_revision,state FROM documents WHERE account_id=$1 AND message_id COLLATE \"C\">$2 AND state<>'deleted' ORDER BY message_id COLLATE \"C\" LIMIT 64",[account,&cursor],fetch_all);
            for (id, generation, completed, state) in &page {
                if completed == revision {
                    write!(local,tx,"UPDATE documents SET embedding_state=embedding_state WHERE account_id=? AND message_id=? AND embedding_revision=?",[account,id,revision],"UPDATE documents SET embedding_state=embedding_state WHERE account_id=$1 AND message_id=$2 AND embedding_revision=$3",[account,id,revision]);
                    continue;
                }
                let state = if matches!(state.as_str(), "pending" | "fetch_error" | "quota") {
                    "waiting"
                } else {
                    "pending"
                };
                write!(local,tx,"INSERT INTO embedding_jobs(account_id,message_id,model_revision,content_version,state) VALUES(?,?,?,?,?) ON CONFLICT(account_id,message_id) DO UPDATE SET model_revision=excluded.model_revision,content_version=excluded.content_version,field=0,byte_offset=0,ordinal=0,state=excluded.state,lease_token=NULL,lease_until=0,retry_at=0,retries=0,error=NULL WHERE embedding_jobs.model_revision<>excluded.model_revision OR embedding_jobs.content_version<>excluded.content_version",[account,id,revision,*generation,state],"INSERT INTO embedding_jobs(account_id,message_id,model_revision,content_version,state) VALUES($1,$2,$3,$4,$5) ON CONFLICT(account_id,message_id) DO UPDATE SET model_revision=excluded.model_revision,content_version=excluded.content_version,field=0,byte_offset=0,ordinal=0,state=excluded.state,lease_token=NULL,lease_until=0,retry_at=0,retries=0,error=NULL WHERE embedding_jobs.model_revision<>excluded.model_revision OR embedding_jobs.content_version<>excluded.content_version",[account,id,revision,*generation,state]);
            }
            let next = page.last().map(|row| row.0.as_str()).unwrap_or(&cursor);
            write!(local,tx,"UPDATE embedding_accounts SET sweep_cursor=?,sweep_done=? WHERE account_id=? AND model_revision=?",[next,i64::from(page.len()<64),account,revision],"UPDATE embedding_accounts SET sweep_cursor=$1,sweep_done=$2 WHERE account_id=$3 AND model_revision=$4",[next,i64::from(page.len()<64),account,revision]);
            tx.commit().await?;
            Ok(!page.is_empty())
        })
    }
    pub async fn claim_embedding(
        &self,
        account: &str,
        revision: &str,
        oldest: bool,
    ) -> Result<Option<EmbeddingTurn>> {
        let now = Utc::now().timestamp();
        let lease = Uuid::new_v4().to_string();
        transaction!(self, account, tx, local, {
            // Retry promotion and abandoned-lease recovery are bounded separately
            // from pending priority selection, avoiding a sort of the backlog.
            write!(local,tx,"UPDATE embedding_jobs SET state='pending',lease_token=NULL,lease_until=0 WHERE queue_id IN (SELECT queue_id FROM embedding_jobs WHERE account_id=? AND ((state='running' AND lease_until<=?) OR (state='retry' AND retry_at<=?)) ORDER BY queue_id LIMIT 32)",[account,now,now],"UPDATE embedding_jobs SET state='pending',lease_token=NULL,lease_until=0 WHERE queue_id IN (SELECT queue_id FROM embedding_jobs WHERE account_id=$1 AND ((state='running' AND lease_until<=$2) OR (state='retry' AND retry_at<=$2)) ORDER BY queue_id LIMIT 32)",[account,now]);
            let order = if oldest {
                "j.last_claim ASC,j.queue_id ASC"
            } else {
                "j.queue_id DESC"
            };
            // Drive the ordered queue; a document-first plan sorts the whole account per claim.
            let index = if oldest {
                "embedding_fair_history"
            } else {
                "embedding_pending"
            };
            let sqlite_claim = format!("SELECT j.message_id,j.content_version,j.field,j.byte_offset,j.ordinal,d.source FROM embedding_jobs j INDEXED BY {index} CROSS JOIN documents d ON d.account_id=j.account_id AND d.message_id=j.message_id CROSS JOIN embedding_accounts a ON a.account_id=j.account_id WHERE j.account_id=? AND j.model_revision=? AND a.model_revision=j.model_revision AND a.model_state='ready' AND j.content_version=d.content_version AND j.state='pending' AND d.state NOT IN ('pending','fetch_error','quota','deleted') ORDER BY {order} LIMIT 1");
            #[cfg(test)]
            if local {
                let plan: Vec<(i64, i64, i64, String)> =
                    sqlx::query_as(&format!("EXPLAIN QUERY PLAN {sqlite_claim}"))
                        .bind(account)
                        .bind(revision)
                        .fetch_all(&mut *tx)
                        .await?;
                assert!(
                    plan.iter().any(|row| row.3.contains(&format!(
                        "INDEX {index} (account_id=? AND model_revision=? AND state=?)"
                    ))),
                    "{plan:?}"
                );
                assert!(
                    !plan.iter().any(|row| row.3.contains("TEMP B-TREE")),
                    "{plan:?}"
                );
            }
            type ClaimRow = (String, i64, i64, i64, i64, String);
            let row:Option<ClaimRow>=rows!(local,tx,&sqlite_claim,[account,revision],&format!("SELECT j.message_id,j.content_version,j.field,j.byte_offset,j.ordinal,d.source FROM embedding_jobs j JOIN documents d ON d.account_id=j.account_id AND d.message_id=j.message_id JOIN embedding_accounts a ON a.account_id=j.account_id WHERE j.account_id=$1 AND j.model_revision=$2 AND a.model_revision=j.model_revision AND a.model_state='ready' AND j.content_version=d.content_version AND j.state='pending' AND d.state NOT IN ('pending','fetch_error','quota','deleted') ORDER BY {order} LIMIT 1"),[account,revision],fetch_optional);
            let Some((id, generation, mut field, mut offset, ordinal, source)) = row else {
                return Ok(None);
            };
            if !(0..=3).contains(&field) || offset < 0 {
                return Err(Error::Invalid);
            }
            while field < 3 {
                let expression = match field {
                    0 => {
                        if local {
                            "sender||char(10)||recipients||char(10)||subject||char(10)||participants"
                        } else {
                            "sender||chr(10)||recipients||chr(10)||subject||chr(10)||participants"
                        }
                    }
                    1 => "body",
                    _ => "attachment_text",
                };
                let (bytes,total):(Vec<u8>,i64)=rows!(local,tx,&format!("SELECT substr(CAST(({expression}) AS BLOB),?,?),length(CAST(({expression}) AS BLOB)) FROM documents WHERE account_id=? AND message_id=? AND content_version=?"),[offset+1,WINDOW as i64,account,&id,generation],&format!("SELECT substring(convert_to(({expression}),'UTF8') FROM ($1::BIGINT)::INTEGER FOR ($2::BIGINT)::INTEGER),octet_length(({expression}))::BIGINT FROM documents WHERE account_id=$3 AND message_id=$4 AND content_version=$5"),[offset+1,WINDOW as i64,account,&id,generation],fetch_one);
                if offset >= total {
                    field += 1;
                    offset = 0;
                    continue;
                }
                let text = utf8_window(bytes)?;
                if text.is_empty() {
                    return Err(Error::Invalid);
                }
                let claim_order: i64 = query!(
                    local,
                    tx,
                    "UPDATE embedding_clock SET value=value+1 WHERE id=1 RETURNING value",
                    [],
                    "UPDATE embedding_clock SET value=value+1 WHERE id=1 RETURNING value",
                    [],
                    fetch_one
                );
                write!(local,tx,"UPDATE embedding_jobs SET field=?,byte_offset=?,state='running',lease_token=?,lease_until=?,last_claim=? WHERE account_id=? AND message_id=?",[field,offset,&lease,now+30,claim_order,account,&id],"UPDATE embedding_jobs SET field=$1,byte_offset=$2,state='running',lease_token=$3,lease_until=$4,last_claim=$5 WHERE account_id=$6 AND message_id=$7",[field,offset,&lease,now+30,claim_order,account,&id]);
                // First work for this content/model discards obsolete vectors.
                if ordinal == 0 {
                    write!(
                        local,
                        tx,
                        "DELETE FROM chunks WHERE account_id=? AND message_id=?",
                        [account, &id],
                        "DELETE FROM chunks WHERE account_id=$1 AND message_id=$2",
                        [account, &id]
                    );
                }
                tx.commit().await?;
                return Ok(Some(EmbeddingTurn {
                    account_id: account.into(),
                    message_id: id,
                    model_revision: revision.into(),
                    content_version: generation,
                    lease_token: lease,
                    field,
                    byte_offset: offset,
                    ordinal,
                    text,
                    total_bytes: total,
                    source,
                }));
            }
            // All fields can be empty. Processing completion is still durable.
            write!(local,tx,"UPDATE documents SET embedding_revision=?,embedding_state='ready',embedding_error=NULL WHERE account_id=? AND message_id=? AND content_version=?",[revision,account,&id,generation],"UPDATE documents SET embedding_revision=$1,embedding_state='ready',embedding_error=NULL WHERE account_id=$2 AND message_id=$3 AND content_version=$4",[revision,account,&id,generation]);
            write!(
                local,
                tx,
                "DELETE FROM embedding_jobs WHERE account_id=? AND message_id=?",
                [account, &id],
                "DELETE FROM embedding_jobs WHERE account_id=$1 AND message_id=$2",
                [account, &id]
            );
            tx.commit().await?;
            Ok(None)
        })
    }
}
impl SearchDatabase {
    pub async fn commit_embedding(
        &self,
        turn: &EmbeddingTurn,
        passage: &EmbeddedPassage,
    ) -> Result<bool> {
        if passage.next_offset == 0
            || passage.next_offset > passage.byte_end
            || passage.byte_end > turn.text.len()
            || !turn.text.is_char_boundary(passage.next_offset)
            || !turn.text.is_char_boundary(passage.byte_end)
            || passage.text != turn.text[..passage.byte_end]
            || (passage.vector.is_none() && !passage.text.trim().is_empty())
        {
            return Err(Error::Invalid);
        }
        if let Some(vector) = &passage.vector {
            let norm = vector
                .iter()
                .map(|v| f64::from(*v) * f64::from(*v))
                .sum::<f64>();
            if vector.len() != 384
                || vector.iter().any(|v| !v.is_finite())
                || (norm - 1.).abs() > 0.005
            {
                return Err(Error::Invalid);
            }
        }
        let account = turn.account_id.as_str();
        let id = turn.message_id.as_str();
        let now = Utc::now().timestamp();
        transaction!(self, account, tx, local, {
            let valid:bool=query!(local,tx,"SELECT EXISTS(SELECT 1 FROM embedding_jobs j JOIN embedding_accounts a ON a.account_id=j.account_id JOIN documents d ON d.account_id=j.account_id AND d.message_id=j.message_id WHERE j.account_id=? AND j.message_id=? AND j.model_revision=? AND a.model_revision=j.model_revision AND j.content_version=? AND d.content_version=j.content_version AND d.state<>'deleted' AND j.lease_token=? AND j.state='running' AND j.lease_until>? AND j.field=? AND j.byte_offset=? AND j.ordinal=?)",[account,id,&turn.model_revision,turn.content_version,&turn.lease_token,now,turn.field,turn.byte_offset,turn.ordinal],"SELECT EXISTS(SELECT 1 FROM embedding_jobs j JOIN embedding_accounts a ON a.account_id=j.account_id JOIN documents d ON d.account_id=j.account_id AND d.message_id=j.message_id WHERE j.account_id=$1 AND j.message_id=$2 AND j.model_revision=$3 AND a.model_revision=j.model_revision AND j.content_version=$4 AND d.content_version=j.content_version AND d.state<>'deleted' AND j.lease_token=$5 AND j.state='running' AND j.lease_until>$6 AND j.field=$7 AND j.byte_offset=$8 AND j.ordinal=$9)",[account,id,&turn.model_revision,turn.content_version,&turn.lease_token,now,turn.field,turn.byte_offset,turn.ordinal],fetch_one);
            if !valid {
                return Ok(false);
            }
            let byte_end = turn
                .byte_offset
                .checked_add(passage.byte_end as i64)
                .ok_or(Error::Invalid)?;
            let next = turn
                .byte_offset
                .checked_add(passage.next_offset as i64)
                .ok_or(Error::Invalid)?;
            if next > turn.total_bytes || byte_end > turn.total_bytes {
                return Err(Error::Invalid);
            }
            let ordinal = turn.ordinal + i64::from(passage.vector.is_some());
            if let Some(vector) = &passage.vector {
                if local {
                    let bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
                    sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field,byte_start,byte_end) VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(account).bind(id).bind(&turn.source).bind(&turn.model_revision).bind(turn.ordinal).bind(&passage.text).bind(&bytes).bind(turn.content_version).bind(turn.field).bind(turn.byte_offset).bind(byte_end).execute(&mut *tx).await?;
                } else {
                    let vector = format!(
                        "[{}]",
                        vector
                            .iter()
                            .map(f32::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    sqlx::query("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version,field,byte_start,byte_end) VALUES($1,$2,$3,$4,$5,$6,$7::vector,$8,$9,$10,$11)").bind(account).bind(id).bind(&turn.source).bind(&turn.model_revision).bind(turn.ordinal).bind(&passage.text).bind(&vector).bind(turn.content_version).bind(turn.field).bind(turn.byte_offset).bind(byte_end).execute(&mut *tx).await?;
                }
            }
            let mut field = turn.field + i64::from(next >= turn.total_bytes);
            let offset = if next >= turn.total_bytes { 0 } else { next };
            // Finish empty remaining fields in this checkpoint. Returning idle
            // from a later claim would add a one-second pause between messages.
            while field < 3 && offset == 0 {
                let column = if field == 1 {
                    "body"
                } else {
                    "attachment_text"
                };
                let length:i64=query!(local,tx,&format!("SELECT length(CAST({column} AS BLOB)) FROM documents WHERE account_id=? AND message_id=?"),[account,id],&format!("SELECT octet_length({column})::BIGINT FROM documents WHERE account_id=$1 AND message_id=$2"),[account,id],fetch_one);
                if length > 0 {
                    break;
                }
                field += 1;
            }
            if field >= 3 {
                write!(local,tx,"UPDATE documents SET embedding_revision=?,embedding_state='ready',embedding_error=NULL WHERE account_id=? AND message_id=? AND content_version=?",[&turn.model_revision,account,id,turn.content_version],"UPDATE documents SET embedding_revision=$1,embedding_state='ready',embedding_error=NULL WHERE account_id=$2 AND message_id=$3 AND content_version=$4",[&turn.model_revision,account,id,turn.content_version]);
                write!(
                    local,
                    tx,
                    "DELETE FROM embedding_jobs WHERE account_id=? AND message_id=?",
                    [account, id],
                    "DELETE FROM embedding_jobs WHERE account_id=$1 AND message_id=$2",
                    [account, id]
                );
            } else {
                write!(local,tx,"UPDATE embedding_jobs SET field=?,byte_offset=?,ordinal=?,state='pending',lease_token=NULL,lease_until=0,retries=0,retry_at=0,error=NULL WHERE account_id=? AND message_id=?",[field,offset,ordinal,account,id],"UPDATE embedding_jobs SET field=$1,byte_offset=$2,ordinal=$3,state='pending',lease_token=NULL,lease_until=0,retries=0,retry_at=0,error=NULL WHERE account_id=$4 AND message_id=$5",[field,offset,ordinal,account,id]);
            }
            tx.commit().await?;
            Ok(true)
        })
    }
    pub async fn defer_embedding(&self, turn: &EmbeddingTurn, reason: &str) -> Result<bool> {
        if !matches!(
            reason,
            "model_failed" | "model_unavailable" | "resource_blocked" | "storage_failed"
        ) {
            return Err(Error::Invalid);
        }
        let account = turn.account_id.as_str();
        transaction!(self, account, tx, local, {
            let retries:Option<i64>=query!(local,tx,"SELECT j.retries FROM embedding_jobs j JOIN embedding_accounts a ON a.account_id=j.account_id WHERE j.account_id=? AND j.message_id=? AND j.model_revision=? AND a.model_revision=j.model_revision AND j.content_version=? AND j.lease_token=? AND j.state='running' AND j.lease_until>?",[account,&turn.message_id,&turn.model_revision,turn.content_version,&turn.lease_token,Utc::now().timestamp()],"SELECT j.retries FROM embedding_jobs j JOIN embedding_accounts a ON a.account_id=j.account_id WHERE j.account_id=$1 AND j.message_id=$2 AND j.model_revision=$3 AND a.model_revision=j.model_revision AND j.content_version=$4 AND j.lease_token=$5 AND j.state='running' AND j.lease_until>$6",[account,&turn.message_id,&turn.model_revision,turn.content_version,&turn.lease_token,Utc::now().timestamp()],fetch_optional);
            let Some(retries) = retries else {
                return Ok(false);
            };
            let delay = (5i64 * 2i64.pow(retries.clamp(0, 10) as u32)).min(3600);
            write!(local,tx,"UPDATE embedding_jobs SET state='retry',retry_at=?,retries=retries+1,error=?,lease_token=NULL,lease_until=0 WHERE account_id=? AND message_id=?",[Utc::now().timestamp()+delay,reason,account,&turn.message_id],"UPDATE embedding_jobs SET state='retry',retry_at=$1,retries=retries+1,error=$2,lease_token=NULL,lease_until=0 WHERE account_id=$3 AND message_id=$4",[Utc::now().timestamp()+delay,reason,account,&turn.message_id]);
            tx.commit().await?;
            Ok(true)
        })
    }
    pub async fn embedding_coverage(&self, account: &str) -> Result<EmbeddingCoverage> {
        type CoverageRow = (String, String, i64, i64, i64, i64, i64);
        let row:Option<CoverageRow>=match self {
            Self::Sqlite(db)=>sqlx::query_as("SELECT a.model_revision,a.model_state,a.sweep_done,a.pending,COALESCE(p.complete,0),COALESCE(p.gaps,0),a.retrying FROM embedding_accounts a LEFT JOIN embedding_progress p ON p.account_id=a.account_id AND p.model_revision=a.model_revision WHERE a.account_id=?").bind(account).fetch_optional(db).await?,
            Self::Postgres(db)=>sqlx::query_as("SELECT a.model_revision,a.model_state,a.sweep_done,a.pending,COALESCE(p.complete,0),COALESCE(p.gaps,0),a.retrying FROM embedding_accounts a LEFT JOIN embedding_progress p ON p.account_id=a.account_id AND p.model_revision=a.model_revision WHERE a.account_id=$1").bind(account).fetch_optional(db).await?,
        };
        Ok(row
            .map(
                |(revision, state, done, pending, processed, gaps, retrying)| EmbeddingCoverage {
                    model_revision: Some(revision),
                    model_available: state == "ready",
                    discovery_complete: done == 1,
                    pending,
                    processed,
                    gaps,
                    retrying,
                },
            )
            .unwrap_or_default())
    }
}
impl SearchConnection {
    pub async fn prepare_embeddings(
        &self,
        account: &str,
        revision: &str,
        available: bool,
    ) -> Result<()> {
        self.connection()
            .await?
            .prepare_embeddings(account, revision, available)
            .await
    }
    pub async fn discover_embeddings(&self, account: &str, revision: &str) -> Result<bool> {
        self.connection()
            .await?
            .discover_embeddings(account, revision)
            .await
    }
    pub async fn claim_embedding(
        &self,
        account: &str,
        revision: &str,
        oldest: bool,
    ) -> Result<Option<EmbeddingTurn>> {
        self.connection()
            .await?
            .claim_embedding(account, revision, oldest)
            .await
    }
    pub async fn commit_embedding(
        &self,
        turn: &EmbeddingTurn,
        passage: &EmbeddedPassage,
    ) -> Result<bool> {
        self.connection()
            .await?
            .commit_embedding(turn, passage)
            .await
    }
    pub async fn defer_embedding(&self, turn: &EmbeddingTurn, reason: &str) -> Result<bool> {
        self.connection().await?.defer_embedding(turn, reason).await
    }
    pub async fn embedding_coverage(&self, account: &str) -> Result<EmbeddingCoverage> {
        self.connection().await?.embedding_coverage(account).await
    }
}
impl crate::Store {
    pub async fn prepare_embeddings(
        &self,
        a: &crate::AuthorizedAccount,
        revision: &str,
        available: bool,
    ) -> Result<()> {
        let _guard = self.gate.read().await;
        if !revision_valid(revision)
            || self
                .expected_search_model
                .as_deref()
                .is_some_and(|r| r != revision)
        {
            return Err(Error::Invalid);
        }
        let target = self.search_database.as_ref().ok_or(Error::Invalid)?;
        self.account_repository(a)
            .await?
            .semantic_profile(revision, available)
            .await?;
        target.prepare_embeddings(a.id(), revision, available).await
    }
    pub async fn discover_embeddings(
        &self,
        a: &crate::AuthorizedAccount,
        revision: &str,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        self.search_database
            .as_ref()
            .ok_or(Error::Invalid)?
            .discover_embeddings(a.id(), revision)
            .await
    }
    pub async fn claim_embedding(
        &self,
        a: &crate::AuthorizedAccount,
        revision: &str,
        oldest: bool,
    ) -> Result<Option<EmbeddingTurn>> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        self.search_database
            .as_ref()
            .ok_or(Error::Invalid)?
            .claim_embedding(a.id(), revision, oldest)
            .await
    }
    pub async fn commit_embedding(
        &self,
        a: &crate::AuthorizedAccount,
        turn: &EmbeddingTurn,
        passage: &EmbeddedPassage,
    ) -> Result<bool> {
        if turn.account_id != a.id() {
            return Err(Error::NotFound);
        }
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        self.search_database
            .as_ref()
            .ok_or(Error::Invalid)?
            .commit_embedding(turn, passage)
            .await
    }
    pub async fn defer_embedding(
        &self,
        a: &crate::AuthorizedAccount,
        turn: &EmbeddingTurn,
        reason: &str,
    ) -> Result<bool> {
        if turn.account_id != a.id() {
            return Err(Error::NotFound);
        }
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        self.search_database
            .as_ref()
            .ok_or(Error::Invalid)?
            .defer_embedding(turn, reason)
            .await
    }
    pub async fn embedding_coverage(
        &self,
        a: &crate::AuthorizedAccount,
    ) -> Result<EmbeddingCoverage> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        self.search_database
            .as_ref()
            .ok_or(Error::Invalid)?
            .embedding_coverage(a.id())
            .await
    }
}
