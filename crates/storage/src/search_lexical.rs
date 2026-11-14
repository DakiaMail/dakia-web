//! Bounded lexical work, independent of model availability. Each turn commits
//! canonical chunks and its continuation together under the account writer lock.
use super::*;
use crate::lexical_text::{self, REVISION, WINDOW};
const FIELDS: [&str; 6] = [
    "sender",
    "recipients",
    "subject",
    "body",
    "participants",
    "attachment_text",
];
type Job = (String, i64, i64, i64, i64, String, i64, i64);
fn sqlite_job(oldest: bool) -> String {
    let (index, order) = if oldest {
        ("lexical_history", "j.last_claim,j.queue_id")
    } else {
        ("lexical_priority", "j.queue_id DESC")
    };
    format!("SELECT j.message_id,j.content_version,j.revision,j.field,j.byte_offset,j.carry,j.skipping,j.ordinal FROM lexical_jobs j INDEXED BY {index} CROSS JOIN documents d INDEXED BY documents_lexical_score ON d.account_id=j.account_id AND d.message_id=j.message_id WHERE j.account_id=? AND j.content_version=d.content_version AND d.state<>'deleted' ORDER BY {order} LIMIT 1")
}

fn window(mut bytes: Vec<u8>) -> Result<String> {
    if let Err(e) = std::str::from_utf8(&bytes) {
        if e.error_len().is_some() {
            return Err(Error::Invalid);
        }
        bytes.truncate(e.valid_up_to());
    }
    String::from_utf8(bytes).map_err(|_| Error::Invalid)
}
impl SearchDatabase {
    pub async fn lexical_turn(&self, account: &str, oldest: bool) -> Result<bool> {
        transaction!(self, account, tx, local, {
            write!(
                local,
                tx,
                "INSERT INTO lexical_accounts(account_id) VALUES(?) ON CONFLICT DO NOTHING",
                [account],
                "INSERT INTO lexical_accounts(account_id) VALUES($1) ON CONFLICT DO NOTHING",
                [account]
            );
            let retry_at: i64 = query!(
                local,
                tx,
                "SELECT retry_at FROM lexical_accounts WHERE account_id=?",
                [account],
                "SELECT retry_at FROM lexical_accounts WHERE account_id=$1",
                [account],
                fetch_one
            );
            if retry_at > chrono::Utc::now().timestamp() {
                tx.commit().await?;
                return Ok(false);
            }
            let (revision, cursor, done): (i64, String, i64) = rows!(
                local,
                tx,
                "SELECT revision,cursor,sweep_done FROM lexical_accounts WHERE account_id=?",
                [account],
                "SELECT revision,cursor,sweep_done FROM lexical_accounts WHERE account_id=$1",
                [account],
                fetch_one
            );
            if revision != REVISION {
                return Err(Error::Invalid);
            }
            let mut discovered = false;
            if done == 0 {
                let page:Vec<(String,i64,i64)>=rows!(local,tx,"SELECT message_id,content_version,lexical_revision FROM documents WHERE account_id=? AND message_id>? AND state<>'deleted' ORDER BY message_id LIMIT 64",[account,&cursor],"SELECT message_id,content_version,lexical_revision FROM documents WHERE account_id=$1 AND message_id COLLATE \"C\">$2 AND state<>'deleted' ORDER BY message_id COLLATE \"C\" LIMIT 64",[account,&cursor],fetch_all);
                for (id, generation, completed) in &page {
                    if *completed == revision {
                        continue;
                    }
                    write!(local,tx,"INSERT INTO lexical_jobs(account_id,message_id,content_version,revision) VALUES(?,?,?,?) ON CONFLICT DO NOTHING",[account,id,*generation,revision],"INSERT INTO lexical_jobs(account_id,message_id,content_version,revision) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING",[account,id,*generation,revision]);
                }
                let next = page.last().map(|r| r.0.as_str()).unwrap_or(&cursor);
                write!(
                    local,
                    tx,
                    "UPDATE lexical_accounts SET cursor=?,sweep_done=? WHERE account_id=?",
                    [next, i64::from(page.len() < 64), account],
                    "UPDATE lexical_accounts SET cursor=$1,sweep_done=$2 WHERE account_id=$3",
                    [next, i64::from(page.len() < 64), account]
                );
                discovered = !page.is_empty();
            }
            let order = if oldest {
                "j.last_claim,j.queue_id"
            } else {
                "j.queue_id DESC"
            };
            // Bound work and memory while amortizing the durable commit for
            // ordinary messages. A large raw window still ends this turn.
            let mut advanced = discovered;
            for _ in 0..8 {
                let job:Option<Job>=rows!(local,tx,&sqlite_job(oldest),[account],&format!("SELECT j.message_id,j.content_version,j.revision,j.field,j.byte_offset,j.carry,j.skipping,j.ordinal FROM lexical_jobs j JOIN documents d ON d.account_id=j.account_id AND d.message_id=j.message_id WHERE j.account_id=$1 AND j.content_version=d.content_version AND d.state<>'deleted' ORDER BY {order} LIMIT 1"),[account],fetch_optional);
                let Some((
                    id,
                    generation,
                    revision,
                    mut field,
                    mut offset,
                    mut carry,
                    skipping,
                    mut ordinal,
                )) = job
                else {
                    write!(local,tx,"UPDATE lexical_accounts SET retries=0,retry_at=0,error=NULL WHERE account_id=?",[account],"UPDATE lexical_accounts SET retries=0,retry_at=0,error=NULL WHERE account_id=$1",[account]);
                    tx.commit().await?;
                    return Ok(advanced);
                };
                if !(0..6).contains(&field) || offset < 0 {
                    return Err(Error::Invalid);
                }
                if field == 0
                    && offset == 0
                    && ordinal == 0
                    && (*tx).inline_current(account, &id).await?
                {
                    write!(local,tx,"UPDATE lexical_accounts SET clock=clock+1,retries=0,retry_at=0,error=NULL WHERE account_id=?",[account],"UPDATE lexical_accounts SET clock=clock+1,retries=0,retry_at=0,error=NULL WHERE account_id=$1",[account]);
                    advanced = true;
                    continue;
                }
                if field == 0 && offset == 0 && ordinal == 0 {
                    write!(
                        local,
                        tx,
                        "DELETE FROM lexical_chunks WHERE account_id=? AND message_id=? AND (revision<>4 OR content_version<>?)",
                        [account, &id,generation],
                        "DELETE FROM lexical_chunks WHERE account_id=$1 AND message_id=$2 AND (revision<>4 OR content_version<>$3)",
                        [account, &id,generation]
                    );
                }
                let column = FIELDS[field as usize];
                let (bytes,total):(Vec<u8>,i64)=rows!(local,tx,&format!("SELECT substr(CAST({column} AS BLOB),?,?),length(CAST({column} AS BLOB)) FROM documents WHERE account_id=? AND message_id=?"),[offset+1,WINDOW as i64,account,&id],&format!("SELECT substring(convert_to({column},'UTF8') FROM ($1::BIGINT)::INTEGER FOR ($2::BIGINT)::INTEGER),octet_length({column})::BIGINT FROM documents WHERE account_id=$3 AND message_id=$4"),[offset+1,WINDOW as i64,account,&id],fetch_one);
                let text = window(bytes)?;
                let finished = offset + text.len() as i64 >= total;
                let step = tokio::task::spawn_blocking(move || {
                    lexical_text::advance(&text, finished, skipping != 0, &carry)
                })
                .await
                .map_err(|_| Error::Invalid)??;
                for chunk in step.chunks {
                    let (native, safe) =
                        lexical_text::native_document(account, field as usize, &chunk);
                    write!(local,tx,"INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(account_id,message_id,field,ordinal) DO UPDATE SET content_version=excluded.content_version,revision=excluded.revision,text=excluded.text,native_text=excluded.native_text,phrase_safe=excluded.phrase_safe",[account,&id,generation,revision,field,ordinal,&chunk,&native,safe],"INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(account_id,message_id,field,ordinal) DO UPDATE SET content_version=excluded.content_version,revision=excluded.revision,text=excluded.text,native_text=excluded.native_text,phrase_safe=excluded.phrase_safe",[account,&id,generation,revision,field,ordinal,&chunk,&native,safe]);
                    ordinal += 1;
                }
                offset += step.consumed as i64;
                carry = step.carry;
                let mut skipping = i64::from(step.skipping);
                if offset >= total {
                    field += 1;
                    offset = 0;
                    ordinal = 0;
                    carry.clear();
                    skipping = 0;
                }
                write!(
                    local,
                    tx,
                    "UPDATE lexical_accounts SET clock=clock+1 WHERE account_id=?",
                    [account],
                    "UPDATE lexical_accounts SET clock=clock+1 WHERE account_id=$1",
                    [account]
                );
                if field == 6 {
                    write!(local,tx,"DELETE FROM lexical_chunks WHERE account_id=? AND message_id=? AND revision<>?",[account,&id,revision],"DELETE FROM lexical_chunks WHERE account_id=$1 AND message_id=$2 AND revision<>$3",[account,&id,revision]);
                    write!(local,tx,"UPDATE documents SET lexical_revision=? WHERE account_id=? AND message_id=? AND content_version=?",[revision,account,&id,generation],"UPDATE documents SET lexical_revision=$1 WHERE account_id=$2 AND message_id=$3 AND content_version=$4",[revision,account,&id,generation]);
                    write!(
                        local,
                        tx,
                        "DELETE FROM lexical_jobs WHERE account_id=? AND message_id=?",
                        [account, &id],
                        "DELETE FROM lexical_jobs WHERE account_id=$1 AND message_id=$2",
                        [account, &id]
                    );
                } else {
                    write!(local,tx,"UPDATE lexical_jobs SET field=?,byte_offset=?,carry=?,skipping=?,ordinal=?,last_claim=(SELECT clock FROM lexical_accounts WHERE account_id=?) WHERE account_id=? AND message_id=?",[field,offset,&carry,skipping,ordinal,account,account,&id],"UPDATE lexical_jobs SET field=$1,byte_offset=$2,carry=$3,skipping=$4,ordinal=$5,last_claim=(SELECT clock FROM lexical_accounts WHERE account_id=$6) WHERE account_id=$6 AND message_id=$7",[field,offset,&carry,skipping,ordinal,account,&id]);
                }
                write!(
                local,
                tx,
                "UPDATE lexical_accounts SET retries=0,retry_at=0,error=NULL WHERE account_id=?",
                [account],
                "UPDATE lexical_accounts SET retries=0,retry_at=0,error=NULL WHERE account_id=$1",
                [account]
            );
                tx.commit().await?;
                return Ok(true);
            }
            tx.commit().await?;
            Ok(advanced)
        })
    }
}
impl SearchConnection {
    pub async fn lexical_turn(&self, account: &str, oldest: bool) -> Result<bool> {
        let db = self.connection().await?;
        let result = db.lexical_turn(account, oldest).await;
        if result.is_err() {
            let _ = db.lexical_failure(account).await;
        }
        result
    }
}
impl crate::Store {
    pub async fn advance_search_lexical(
        &self,
        a: &crate::AuthorizedAccount,
        oldest: bool,
    ) -> Result<bool> {
        let _guard = self.gate.read().await;
        self.account_repository(a).await?;
        self.search_database
            .as_ref()
            .ok_or(Error::Invalid)?
            .lexical_turn(a.id(), oldest)
            .await
    }
}

// Keep normal messages immediately searchable within their publication commit.
// Larger retained documents are handled by bounded background work above.
macro_rules! inline {
    ($name:ident,$db:ty,$read:expr,$insert:expr,$finish:expr,$delete:expr,$trim:expr) => {
        pub(super) async fn $name(db: &mut $db, account: &str, id: &str) -> Result<bool> {
            let raw: Option<(i64, i64, String, String, String, String, String, String)> =
                sqlx::query_as($read)
                    .bind(account)
                    .bind(id)
                    .bind(WINDOW as i64)
                    .fetch_optional(&mut *db)
                    .await?;
            let Some((
                generation,
                revision,
                sender,
                recipients,
                subject,
                body,
                participants,
                attachments,
            )) = raw
            else {
                return Ok(false);
            };
            if revision != REVISION {
                return Err(Error::Invalid);
            }
            let chunks = tokio::task::spawn_blocking(move || {
                let mut chunks = vec![];
                for (field, text) in [sender, recipients, subject, body, participants, attachments]
                    .into_iter()
                    .enumerate()
                {
                    let step = lexical_text::advance(&text, true, false, "")?;
                    for (ordinal, chunk) in step.chunks.into_iter().enumerate() {
                        chunks.push((field as i64, ordinal as i64, chunk));
                    }
                }
                Ok::<_, Error>(chunks)
            })
            .await
            .map_err(|_| Error::Invalid)??;
            let keys=serde_json::to_string(&chunks.iter().map(|(field,ordinal,_)|serde_json::json!({"field":field,"ordinal":ordinal})).collect::<Vec<_>>())?;
            for (field, ordinal, text) in chunks {
                let (native,safe)=lexical_text::native_document(account,field as usize,&text);
                sqlx::query($insert)
                    .bind(account)
                    .bind(id)
                    .bind(generation)
                    .bind(revision)
                    .bind(field)
                    .bind(ordinal)
                    .bind(text).bind(native).bind(safe)
                    .execute(&mut *db)
                    .await?;
            }
            sqlx::query($trim).bind(account).bind(id).bind(keys).execute(&mut *db).await?;
            sqlx::query($finish)
                .bind(account)
                .bind(id)
                .bind(revision)
                .execute(&mut *db)
                .await?;
            sqlx::query($delete)
                .bind(account)
                .bind(id)
                .execute(&mut *db)
                .await?;
            Ok(true)
        }
        impl InlineLexical for $db {
            async fn inline_current(&mut self,account:&str,id:&str)->Result<bool> { $name(self,account,id).await }
        }
    };
}
trait InlineLexical {
    async fn inline_current(&mut self, account: &str, id: &str) -> Result<bool>;
}
inline!(inline_sqlite,sqlx::SqliteConnection,
 "SELECT d.content_version,a.revision,d.sender,d.recipients,d.subject,d.body,d.participants,d.attachment_text FROM documents d JOIN lexical_accounts a ON a.account_id=d.account_id JOIN lexical_jobs j ON j.account_id=d.account_id AND j.message_id=d.message_id WHERE d.account_id=?1 AND d.message_id=?2 AND d.state<>'deleted' AND d.lexical_revision<>a.revision AND j.field=0 AND j.byte_offset=0 AND length(CAST(d.sender||d.recipients||d.subject||d.body||d.participants||d.attachment_text AS BLOB))<=?3",
 "INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT(account_id,message_id,field,ordinal) DO UPDATE SET content_version=excluded.content_version,revision=excluded.revision,text=excluded.text,native_text=excluded.native_text,phrase_safe=excluded.phrase_safe",
 "UPDATE documents SET lexical_revision=?3 WHERE account_id=?1 AND message_id=?2",
 "DELETE FROM lexical_jobs WHERE account_id=?1 AND message_id=?2",
 "DELETE FROM lexical_chunks WHERE account_id=?1 AND message_id=?2 AND NOT EXISTS(SELECT 1 FROM json_each(?3) k WHERE json_extract(k.value,'$.field')=lexical_chunks.field AND json_extract(k.value,'$.ordinal')=lexical_chunks.ordinal)");
inline!(inline_postgres,sqlx::PgConnection,
 "SELECT d.content_version,a.revision,d.sender,d.recipients,d.subject,d.body,d.participants,d.attachment_text FROM documents d JOIN lexical_accounts a ON a.account_id=d.account_id JOIN lexical_jobs j ON j.account_id=d.account_id AND j.message_id=d.message_id WHERE d.account_id=$1 AND d.message_id=$2 AND d.state<>'deleted' AND d.lexical_revision<>a.revision AND j.field=0 AND j.byte_offset=0 AND octet_length(d.sender||d.recipients||d.subject||d.body||d.participants||d.attachment_text)::BIGINT<=$3",
 "INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(account_id,message_id,field,ordinal) DO UPDATE SET content_version=excluded.content_version,revision=excluded.revision,text=excluded.text,native_text=excluded.native_text,phrase_safe=excluded.phrase_safe",
 "UPDATE documents SET lexical_revision=$3 WHERE account_id=$1 AND message_id=$2",
 "DELETE FROM lexical_jobs WHERE account_id=$1 AND message_id=$2",
 "DELETE FROM lexical_chunks c WHERE account_id=$1 AND message_id=$2 AND NOT EXISTS(SELECT 1 FROM jsonb_to_recordset($3::jsonb) k(field BIGINT,ordinal BIGINT) WHERE k.field=c.field AND k.ordinal=c.ordinal)");
#[derive(Default)]
pub(crate) struct LexicalCoverage {
    pub pending: i64,
    pub revision: Option<i64>,
    pub failed: bool,
    pub discovered: bool,
}
impl SearchConnection {
    pub async fn lexical_coverage(&self, account: &str) -> Result<LexicalCoverage> {
        let row: Option<(i64, i64, i64, i64)> =
            match self.connection().await? {
                SearchDatabase::Sqlite(db) => sqlx::query_as(
                    "SELECT pending,retries,sweep_done,revision FROM lexical_accounts WHERE account_id=?",
                )
                .bind(account)
                .fetch_optional(&db)
                .await?,
                SearchDatabase::Postgres(db) => sqlx::query_as(
                    "SELECT pending,retries,sweep_done,revision FROM lexical_accounts WHERE account_id=$1",
                )
                .bind(account)
                .fetch_optional(&db)
                .await?,
            };
        Ok(row
            .map(|(pending, retries, done, revision)| LexicalCoverage {
                revision: Some(revision),
                pending,
                failed: retries > 0,
                discovered: done != 0,
            })
            .unwrap_or_default())
    }
}
impl SearchDatabase {
    async fn lexical_failure(&self, account: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        match self {
            Self::Sqlite(db) => {
                sqlx::query("UPDATE lexical_accounts SET retries=retries+1,retry_at=?+min(3600,5*(1<<min(retries,10))),error='indexing_failed' WHERE account_id=?").bind(now).bind(account).execute(db).await?;
            }
            Self::Postgres(db) => {
                sqlx::query("UPDATE lexical_accounts SET retries=retries+1,retry_at=$1+LEAST(3600,5*(1::BIGINT<<LEAST(retries,10)::INTEGER)),error='indexing_failed' WHERE account_id=$2").bind(now).bind(account).execute(db).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod queue_plan_tests {
    use super::*;
    #[tokio::test]
    async fn job_selection_starts_with_priority_and_uses_body_free_identity_lookups() {
        let dir = tempfile::tempdir().unwrap();
        let SearchDatabase::Sqlite(db) = SearchDatabase::open(dir.path(), None).await.unwrap()
        else {
            unreachable!()
        };
        // The exact generated production statement must retain its plan for both
        // fairness lanes, independently of account-size statistics.
        for oldest in [true, false] {
            let plan: Vec<(i64, i64, i64, String)> =
                sqlx::query_as(&format!("EXPLAIN QUERY PLAN {}", sqlite_job(oldest)))
                    .bind("account")
                    .fetch_all(&db)
                    .await
                    .unwrap();
            let index = if oldest {
                "lexical_history"
            } else {
                "lexical_priority"
            };
            assert!(
                plan.iter().any(|row| row
                    .3
                    .contains(&format!("SEARCH j USING INDEX {index} (account_id=?)"))),
                "{plan:?}"
            );
            assert!(plan.iter().any(|row|row.3.contains("SEARCH d USING COVERING INDEX documents_lexical_score (account_id=? AND message_id=?")),"{plan:?}");
            assert!(
                !plan
                    .iter()
                    .any(|row| row.3.contains("TEMP B-TREE") || row.3.starts_with("SCAN d")),
                "{plan:?}"
            );
        }
    }
}
