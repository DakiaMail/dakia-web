//! Account persistence with explicit native SQL. Domain transitions and revision
//! checks are shared; drivers, ownership locks, and transaction details stay here.
use crate::*;
#[cfg(test)]
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::str::FromStr;
#[path = "mailbox_counts.rs"]
mod counts;
#[cfg(test)]
#[path = "account_repository_tests.rs"]
mod tests;

#[derive(Clone)]
pub(crate) struct AccountRepository {
    backend: Backend,
    account: String,
    owner: String,
}
#[derive(Clone)]
enum Backend {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

// Each branch has native SQL and its own bind list. Only typed domain values
// cross the repository boundary; no runtime SQL translation is involved.
macro_rules! scalar {
    ($local:expr,$tx:ident,$sql:expr,[$($arg:expr),*],$pg:expr,[$($p:expr),*],$method:ident) => {
        if $local {
            sqlx::query_scalar($sql)$(.bind($arg))*.$method(&mut *$tx).await?
        } else {
            sqlx::query_scalar($pg)$(.bind($p))*.$method(&mut *$tx).await?
        }
    };
}
macro_rules! execute {
    ($local:expr,$tx:ident,$sql:expr,[$($arg:expr),*],$pg:expr,[$($p:expr),*]) => {
        if $local {
            sqlx::query($sql)$(.bind($arg))*.execute(&mut *$tx).await?.rows_affected()
        } else {
            sqlx::query($pg)$(.bind($p))*.execute(&mut *$tx).await?.rows_affected()
        }
    };
}
macro_rules! transaction {
    ($self:expr,$tx:ident,$local:ident,$write:expr,$body:block) => {
        match &$self.backend {
            Backend::Sqlite(db) => {
                let mut $tx = if $write {
                    db.begin_with("BEGIN IMMEDIATE").await?
                } else {
                    db.begin().await?
                };
                let $local = true;
                $body
            }
            Backend::Postgres(db) => {
                let mut $tx = $self.begin_postgres(db, $write).await?;
                let $local = false;
                $body
            }
        }
    };
}
macro_rules! event {
    ($self:expr,$local:expr,$tx:ident,$revision:expr,$kind:expr,$id:expr) => {{
        if $local {
            sqlx::query("INSERT INTO events(revision,kind,target_id) VALUES(?,?,?)")
                .bind(i64::from($revision)).bind($kind).bind($id).execute(&mut *$tx).await?;
        } else {
            sqlx::query("WITH next AS (UPDATE account_locks SET event_cursor=event_cursor+1 WHERE account_id=$1 RETURNING event_cursor) INSERT INTO events(account_id,cursor,revision,kind,target_id) SELECT $1,event_cursor,$2,$3,$4 FROM next")
                .bind(&$self.account).bind(i64::from($revision)).bind($kind).bind($id).execute(&mut *$tx).await?;
        }
    }};
}

impl Store {
    pub(crate) async fn account_repository(
        &self,
        a: &AuthorizedAccount,
    ) -> Result<AccountRepository> {
        if let Some(application) = &self.application_database {
            if !Arc::ptr_eq(&self.identity, &a.identity) || Uuid::parse_str(&a.id).is_err() {
                return Err(Error::NotFound);
            }
            self.authorize(&a.user_id, &a.id).await?;
            return Ok(application.repository(a));
        }
        Ok(AccountRepository::sqlite(self.db(a).await?, a))
    }
}
impl AccountRepository {
    async fn begin_postgres<'db>(
        &self,
        db: &'db PgPool,
        write: bool,
    ) -> Result<sqlx::Transaction<'db, sqlx::Postgres>> {
        let mut tx = db.begin().await?;
        // Hold current ownership through the operation, including account removal.
        let owned: Option<String> = sqlx::query_scalar(
            "SELECT id FROM dakia_control.accounts WHERE id=$1 AND user_id=$2 FOR KEY SHARE",
        )
        .bind(&self.account)
        .bind(&self.owner)
        .fetch_optional(&mut *tx)
        .await?;
        if owned.is_none() {
            return Err(Error::NotFound);
        }
        if write {
            sqlx::query("INSERT INTO account_locks(account_id,user_id) VALUES($1,$2) ON CONFLICT(account_id) DO NOTHING")
                .bind(&self.account).bind(&self.owner).execute(&mut *tx).await?;
            sqlx::query("UPDATE account_locks SET event_cursor=event_cursor WHERE account_id=$1 AND user_id=$2")
                .bind(&self.account).bind(&self.owner).execute(&mut *tx).await?;
        }
        Ok(tx)
    }
    pub(crate) fn sqlite(db: SqlitePool, a: &AuthorizedAccount) -> Self {
        Self {
            backend: Backend::Sqlite(db),
            account: a.id.clone(),
            owner: a.user_id.clone(),
        }
    }
    #[cfg(test)]
    pub(crate) async fn postgres(url: &str, owner: &str, account: &str) -> Result<Self> {
        let application = ApplicationDatabase::open_postgres(url).await?;
        Ok(Self {
            backend: Backend::Postgres(application.db),
            owner: owner.into(),
            account: account.into(),
        })
    }
    pub(crate) async fn draft(&self, id: &str) -> Result<Draft> {
        transaction!(self, tx, local, false, {
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM drafts WHERE id=?",
                [id],
                "SELECT document FROM drafts WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
        })
    }
    pub(crate) async fn save_draft(&self, mut draft: Draft) -> Result<Draft> {
        if draft.account_id != self.account
            || draft.id.is_empty()
            || draft.subject.contains(['\r', '\n'])
            || draft
                .attachments
                .iter()
                .map(|x| u64::from(x.size_bytes))
                .sum::<u64>()
                > 20 * 1024 * 1024
        {
            return Err(Error::Invalid);
        }
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE drafts SET revision=revision WHERE id=?",
                [&draft.id],
                "UPDATE drafts SET revision=revision WHERE account_id=$1 AND id=$2",
                [&self.account, &draft.id]
            );
            let current: Option<i64> = scalar!(
                local,
                tx,
                "SELECT revision FROM drafts WHERE id=?",
                [&draft.id],
                "SELECT revision FROM drafts WHERE account_id=$1 AND id=$2",
                [&self.account, &draft.id],
                fetch_optional
            );
            if current.unwrap_or(0) != i64::from(draft.revision) {
                return Err(Error::Conflict);
            }
            for attachment in &draft.attachments {
                let exists:bool = scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM blobs WHERE id=? AND size_bytes=?)",[&attachment.id,i64::from(attachment.size_bytes)],"SELECT EXISTS(SELECT 1 FROM blobs WHERE account_id=$1 AND id=$2 AND size_bytes=$3)",[&self.account,&attachment.id,i64::from(attachment.size_bytes)],fetch_one);
                if !exists {
                    return Err(Error::Invalid);
                }
            }
            draft.revision = draft.revision.checked_add(1).ok_or(Error::Invalid)?;
            draft.saved_at = now();
            let raw = serde_json::to_string(&draft)?;
            execute!(local,tx,"INSERT INTO drafts(id,revision,document) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,document=excluded.document",[&draft.id,i64::from(draft.revision),&raw],"INSERT INTO drafts(account_id,id,revision,document) VALUES($1,$2,$3,$4) ON CONFLICT(account_id,id) DO UPDATE SET revision=excluded.revision,document=excluded.document",[&self.account,&draft.id,i64::from(draft.revision),&raw]);
            execute!(
                local,
                tx,
                "DELETE FROM draft_blobs WHERE draft_id=?",
                [&draft.id],
                "DELETE FROM draft_blobs WHERE account_id=$1 AND draft_id=$2",
                [&self.account, &draft.id]
            );
            for attachment in &draft.attachments {
                execute!(local,tx,"INSERT INTO draft_blobs(draft_id,blob_id) VALUES(?,?) ON CONFLICT DO NOTHING",[&draft.id,&attachment.id],"INSERT INTO draft_blobs(account_id,draft_id,blob_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING",[&self.account,&draft.id,&attachment.id]);
                execute!(
                    local,
                    tx,
                    "UPDATE blobs SET pinned=1 WHERE id=?",
                    [&attachment.id],
                    "UPDATE blobs SET pinned=1 WHERE account_id=$1 AND id=$2",
                    [&self.account, &attachment.id]
                );
            }
            event!(self, local, tx, draft.revision, "draft_saved", &draft.id);
            tx.commit().await?;
            Ok(draft)
        })
    }
    pub(crate) async fn submit(&self, request: &SubmitRequest, due: &str) -> Result<Submission> {
        if request.operation_id.is_empty() || request.timezone.is_empty() {
            return Err(Error::Invalid);
        }
        let encoded = serde_json::to_string(request)?;
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE submissions SET state=state WHERE operation_id=?",
                [&request.operation_id],
                "UPDATE submissions SET state=state WHERE account_id=$1 AND operation_id=$2",
                [&self.account, &request.operation_id]
            );
            let existing: Option<(String, String)> = if local {
                sqlx::query_as("SELECT request,document FROM submissions WHERE operation_id=?")
                    .bind(&request.operation_id)
                    .fetch_optional(&mut *tx)
                    .await?
            } else {
                sqlx::query_as("SELECT request,document FROM submissions WHERE account_id=$1 AND operation_id=$2").bind(&self.account).bind(&request.operation_id).fetch_optional(&mut *tx).await?
            };
            if let Some((previous, document)) = existing {
                if previous != encoded {
                    return Err(Error::Conflict);
                }
                return Ok(serde_json::from_str(&document)?);
            }
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM drafts WHERE id=? AND revision=?",
                [&request.draft_id, i64::from(request.expected_revision)],
                "SELECT document FROM drafts WHERE account_id=$1 AND id=$2 AND revision=$3",
                [
                    &self.account,
                    &request.draft_id,
                    i64::from(request.expected_revision)
                ],
                fetch_optional
            );
            let raw = raw.ok_or(Error::Conflict)?;
            let draft: Draft = serde_json::from_str(&raw)?;
            if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
                return Err(Error::Invalid);
            }
            let submission = Submission {
                id: Uuid::new_v4().to_string(),
                account_id: self.account.clone(),
                operation_id: request.operation_id.clone(),
                draft_id: draft.id.clone(),
                draft_revision: draft.revision,
                state: SubmissionState::Scheduled,
                due_at: due.into(),
                timezone: request.timezone.clone(),
                filing_status: FilingStatus::Pending,
                revision: 1,
            };
            let document = serde_json::to_string(&submission)?;
            execute!(local,tx,"INSERT INTO submissions(id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES(?,?,?,?,?,?,?,'scheduled',?,'pending')",[&submission.id,&submission.operation_id,&encoded,&draft.id,i64::from(draft.revision),&raw,&document,due],"INSERT INTO submissions(account_id,id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'scheduled',$9,'pending')",[&self.account,&submission.id,&submission.operation_id,&encoded,&draft.id,i64::from(draft.revision),&raw,&document,due]);
            execute!(
                local,
                tx,
                "UPDATE submissions SET scheduled_by_user=? WHERE id=?",
                [i64::from(request.due_at.is_some()), &submission.id],
                "UPDATE submissions SET scheduled_by_user=$1 WHERE account_id=$2 AND id=$3",
                [
                    i64::from(request.due_at.is_some()),
                    &self.account,
                    &submission.id
                ]
            );
            execute!(
                local,
                tx,
                "INSERT INTO outbox(kind,target_id) VALUES('submit',?)",
                [&submission.id],
                "INSERT INTO outbox(account_id,kind,target_id) VALUES($1,'submit',$2)",
                [&self.account, &submission.id]
            );
            event!(
                self,
                local,
                tx,
                1u32,
                "submission_scheduled",
                &submission.id
            );
            tx.commit().await?;
            Ok(submission)
        })
    }
    pub(crate) async fn submission(&self, id: &str) -> Result<Submission> {
        transaction!(self, tx, local, false, {
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM submissions WHERE id=?",
                [id],
                "SELECT document FROM submissions WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
        })
    }
    pub(crate) async fn transition(
        &self,
        id: &str,
        from: SubmissionState,
        to: SubmissionState,
    ) -> Result<Submission> {
        use SubmissionState::*;
        if !matches!(
            (&from, &to),
            (Scheduled, Claimed)
                | (Scheduled, Cancelled)
                | (Claimed, Cancelled)
                | (Claimed, Submitting)
                | (Claimed, Scheduled)
                | (Claimed, Failed)
                | (Submitting, Accepted)
                | (Submitting, Failed)
                | (Submitting, DeliveryUnknown)
        ) {
            return Err(Error::Invalid);
        }
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE submissions SET state=state WHERE id=?",
                [id],
                "UPDATE submissions SET state=state WHERE account_id=$1 AND id=$2",
                [&self.account, id]
            );
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM submissions WHERE id=?",
                [id],
                "SELECT document FROM submissions WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            let mut submission: Submission = serde_json::from_str(&raw.ok_or(Error::NotFound)?)?;
            if submission.state != from {
                return Err(Error::Conflict);
            }
            submission.state = to;
            submission.revision = submission.revision.checked_add(1).ok_or(Error::Invalid)?;
            let state = wire(&submission.state)?;
            let raw = serde_json::to_string(&submission)?;
            let lease =
                (Utc::now() + Duration::seconds(60)).to_rfc3339_opts(SecondsFormat::Millis, true);
            execute!(local,tx,"UPDATE submissions SET state=?,document=?,lease_until=? WHERE id=?",[&state,&raw,&lease,id],"UPDATE submissions SET state=$1,document=$2,lease_until=$3 WHERE account_id=$4 AND id=$5",[&state,&raw,&lease,&self.account,id]);
            event!(
                self,
                local,
                tx,
                submission.revision,
                "submission_changed",
                id
            );
            tx.commit().await?;
            Ok(submission)
        })
    }
    pub(crate) async fn expired_submissions(&self, at: &str) -> Result<Vec<(String, String)>> {
        transaction!(self, tx, local, false, {
            Ok(if local {
                sqlx::query_as("SELECT id,state FROM submissions WHERE state IN ('claimed','submitting') AND lease_until<=?").bind(at).fetch_all(&mut *tx).await?
            } else {
                sqlx::query_as("SELECT id,state FROM submissions WHERE account_id=$1 AND state IN ('claimed','submitting') AND lease_until<=$2").bind(&self.account).bind(at).fetch_all(&mut *tx).await?
            })
        })
    }
    pub(crate) async fn event_cursor(&self) -> Result<u32> {
        transaction!(self, tx, local, false, {
            let cursor: i64 = scalar!(
                local,
                tx,
                "SELECT COALESCE(MAX(cursor),0) FROM events",
                [],
                "SELECT COALESCE(MAX(cursor),0) FROM events WHERE account_id=$1",
                [&self.account],
                fetch_one
            );
            u32::try_from(cursor).map_err(|_| Error::Invalid)
        })
    }
    pub(crate) async fn events(&self, after: u32) -> Result<Vec<Event>> {
        transaction!(self, tx, local, false, {
            let min: Option<i64> = scalar!(
                local,
                tx,
                "SELECT MIN(cursor) FROM events",
                [],
                "SELECT MIN(cursor) FROM events WHERE account_id=$1",
                [&self.account],
                fetch_one
            );
            if after > 0 && min.is_some_and(|min| i64::from(after) + 1 < min) {
                return Err(Error::CursorExpired);
            }
            let rows: Vec<(i64, i64, String, String)> = if local {
                sqlx::query_as("SELECT cursor,revision,kind,target_id FROM events WHERE cursor>? ORDER BY cursor LIMIT 500").bind(i64::from(after)).fetch_all(&mut *tx).await?
            } else {
                sqlx::query_as("SELECT cursor,revision,kind,target_id FROM events WHERE account_id=$1 AND cursor>$2 ORDER BY cursor LIMIT 500").bind(&self.account).bind(i64::from(after)).fetch_all(&mut *tx).await?
            };
            rows.into_iter()
                .map(|(cursor, revision, kind, target_id)| {
                    Ok(Event {
                        account_id: self.account.clone(),
                        cursor: u32::try_from(cursor).map_err(|_| Error::Invalid)?,
                        revision: u32::try_from(revision).map_err(|_| Error::Invalid)?,
                        kind,
                        target_id,
                    })
                })
                .collect()
        })
    }
}

#[path = "mailbox_transaction.rs"]
pub(crate) mod mailbox;
fn wire(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or(Error::Invalid)
}

impl AccountRepository {
    pub(crate) async fn put_attachment_metadata(&self, attachment: &Attachment) -> Result<()> {
        let raw = serde_json::to_string(attachment)?;
        let at = now();
        transaction!(self, tx, local, true, {
            execute!(local,tx,"INSERT INTO blobs(id,size_bytes,pinned,last_access) VALUES(?,?,1,?)",[&attachment.id,i64::from(attachment.size_bytes),&at],"INSERT INTO blobs(account_id,id,size_bytes,pinned,last_access) VALUES($1,$2,$3,1,$4)",[&self.account,&attachment.id,i64::from(attachment.size_bytes),&at]);
            execute!(
                local,
                tx,
                "INSERT INTO attachment_metadata(id,document) VALUES(?,?)",
                [&attachment.id, &raw],
                "INSERT INTO attachment_metadata(account_id,id,document) VALUES($1,$2,$3)",
                [&self.account, &attachment.id, &raw]
            );
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn attachment_metadata(&self, id: &str) -> Result<Attachment> {
        transaction!(self, tx, local, false, {
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM attachment_metadata WHERE id=?",
                [id],
                "SELECT document FROM attachment_metadata WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
        })
    }
    pub(crate) async fn cleanup_attachment_metadata(&self, cutoff: &str) -> Result<Vec<String>> {
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE blobs SET pinned=pinned WHERE 0",
                [],
                "UPDATE blobs SET pinned=pinned WHERE FALSE",
                []
            );
            let ids:Vec<String>=scalar!(local,tx,"SELECT id FROM blobs WHERE last_access<? AND NOT EXISTS(SELECT 1 FROM draft_blobs WHERE blob_id=blobs.id) AND NOT EXISTS(SELECT 1 FROM submissions,json_each(submissions.snapshot,'$.attachments') part WHERE json_extract(part.value,'$.id')=blobs.id) ORDER BY id LIMIT 100",[cutoff],"SELECT id FROM blobs WHERE account_id=$1 AND last_access<$2 AND NOT EXISTS(SELECT 1 FROM draft_blobs WHERE account_id=$1 AND blob_id=blobs.id) AND NOT EXISTS(SELECT 1 FROM submissions CROSS JOIN LATERAL jsonb_array_elements(snapshot::jsonb->'attachments') part WHERE submissions.account_id=$1 AND part->>'id'=blobs.id) ORDER BY id LIMIT 100",[&self.account,cutoff],fetch_all);
            for id in &ids {
                Uuid::parse_str(id).map_err(|_| Error::Invalid)?;
                execute!(
                    local,
                    tx,
                    "DELETE FROM attachment_metadata WHERE id=?",
                    [id],
                    "DELETE FROM attachment_metadata WHERE account_id=$1 AND id=$2",
                    [&self.account, id]
                );
                execute!(
                    local,
                    tx,
                    "DELETE FROM blobs WHERE id=?",
                    [id],
                    "DELETE FROM blobs WHERE account_id=$1 AND id=$2",
                    [&self.account, id]
                );
            }
            tx.commit().await?;
            Ok(ids)
        })
    }
    pub(crate) async fn attachment_retained(&self, id: &str) -> Result<bool> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT EXISTS(SELECT 1 FROM blobs WHERE id=?) OR EXISTS(SELECT 1 FROM attachment_metadata WHERE id=?) OR EXISTS(SELECT 1 FROM draft_blobs WHERE blob_id=?) OR EXISTS(SELECT 1 FROM submissions,json_each(submissions.snapshot,'$.attachments') part WHERE json_extract(part.value,'$.id')=?)",[id,id,id,id],"SELECT EXISTS(SELECT 1 FROM blobs WHERE account_id=$1 AND id=$2) OR EXISTS(SELECT 1 FROM attachment_metadata WHERE account_id=$1 AND id=$2) OR EXISTS(SELECT 1 FROM draft_blobs WHERE account_id=$1 AND blob_id=$2) OR EXISTS(SELECT 1 FROM submissions CROSS JOIN LATERAL jsonb_array_elements(snapshot::jsonb->'attachments') part WHERE account_id=$1 AND part->>'id'=$2)",[&self.account,id],fetch_one))
        })
    }
}

impl AccountRepository {
    pub(crate) async fn scheduled_messages(&self) -> Result<Vec<ScheduledMessage>> {
        transaction!(self, tx, local, false, {
            let rows: Vec<(String, String)> = if local {
                sqlx::query_as("SELECT document,snapshot FROM submissions WHERE scheduled_by_user=1 AND (state IN ('scheduled','claimed','submitting','failed','delivery_unknown') OR (state='accepted' AND filing_status='failed')) ORDER BY due_at,id").fetch_all(&mut *tx).await?
            } else {
                sqlx::query_as("SELECT document,snapshot FROM submissions WHERE account_id=$1 AND scheduled_by_user=1 AND (state IN ('scheduled','claimed','submitting','failed','delivery_unknown') OR (state='accepted' AND filing_status='failed')) ORDER BY due_at,id").bind(&self.account).fetch_all(&mut *tx).await?
            };
            rows.into_iter()
                .map(|(document, snapshot)| {
                    let draft: Draft = serde_json::from_str(&snapshot)?;
                    Ok(ScheduledMessage {
                        submission: serde_json::from_str(&document)?,
                        subject: draft.subject,
                        to: draft.to,
                    })
                })
                .collect()
        })
    }
    pub(crate) async fn drafts(&self) -> Result<Vec<Draft>> {
        transaction!(self, tx, local, false, {
            let rows:Vec<String>=scalar!(local,tx,"SELECT document FROM drafts ORDER BY json_extract(document,'$.saved_at') DESC,id",[],"SELECT document FROM drafts WHERE account_id=$1 ORDER BY document::jsonb->>'saved_at' DESC,id",[&self.account],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn submissions(&self) -> Result<Vec<Submission>> {
        transaction!(self, tx, local, false, {
            let rows:Vec<String>=scalar!(local,tx,"SELECT document FROM submissions ORDER BY due_at DESC,id LIMIT 200",[],"SELECT document FROM submissions WHERE account_id=$1 ORDER BY due_at DESC,id LIMIT 200",[&self.account],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn due_submission(&self, at: &str) -> Result<Option<String>> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT id FROM submissions WHERE state='scheduled' AND due_at<=? ORDER BY due_at,id LIMIT 1",[at],"SELECT id FROM submissions WHERE account_id=$1 AND state='scheduled' AND due_at<=$2 ORDER BY due_at,id LIMIT 1",[&self.account,at],fetch_optional))
        })
    }
    pub(crate) async fn submission_snapshot(&self, id: &str) -> Result<Draft> {
        transaction!(self, tx, local, false, {
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT snapshot FROM submissions WHERE id=?",
                [id],
                "SELECT snapshot FROM submissions WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            Ok(serde_json::from_str(&raw.ok_or(Error::NotFound)?)?)
        })
    }
    pub(crate) async fn mime(&self, id: &str) -> Result<Option<PersistedMime>> {
        transaction!(self, tx, local, false, {
            let row: Option<(Vec<u8>, String, String)> = if local {
                sqlx::query_as(
                    "SELECT raw,recipients,message_id FROM submission_mime WHERE submission_id=?",
                )
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
            } else {
                sqlx::query_as("SELECT raw,recipients,message_id FROM submission_mime WHERE account_id=$1 AND submission_id=$2").bind(&self.account).bind(id).fetch_optional(&mut *tx).await?
            };
            row.map(|(raw, recipients, message_id)| {
                Ok(PersistedMime {
                    raw,
                    recipients: serde_json::from_str(&recipients)?,
                    message_id,
                })
            })
            .transpose()
        })
    }
    pub(crate) async fn persist_mime(
        &self,
        id: &str,
        mime: PersistedMime,
    ) -> Result<PersistedMime> {
        let recipients = serde_json::to_string(&mime.recipients)?;
        transaction!(self, tx, local, true, {
            execute!(local,tx,"INSERT INTO submission_mime(submission_id,raw,recipients,message_id) VALUES(?,?,?,?) ON CONFLICT DO NOTHING",[id,&mime.raw,&recipients,&mime.message_id],"INSERT INTO submission_mime(account_id,submission_id,raw,recipients,message_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",[&self.account,id,&mime.raw,&recipients,&mime.message_id]);
            tx.commit().await?;
        });
        self.mime(id).await?.ok_or(Error::NotFound)
    }
    pub(crate) async fn needs_filing(&self) -> Result<Vec<Submission>> {
        transaction!(self, tx, local, false, {
            let rows:Vec<String> = scalar!(local,tx,"SELECT document FROM submissions WHERE state='accepted' AND filing_status IN ('pending','failed') ORDER BY due_at LIMIT 10",[],"SELECT document FROM submissions WHERE account_id=$1 AND state='accepted' AND filing_status IN ('pending','failed') ORDER BY due_at LIMIT 10",[&self.account],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn set_filing(&self, id: &str, status: FilingStatus) -> Result<Submission> {
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE submissions SET state=state WHERE id=?",
                [id],
                "UPDATE submissions SET state=state WHERE account_id=$1 AND id=$2",
                [&self.account, id]
            );
            let raw:Option<String> = scalar!(local,tx,"SELECT document FROM submissions WHERE id=? AND state='accepted'",[id],"SELECT document FROM submissions WHERE account_id=$1 AND id=$2 AND state='accepted'",[&self.account,id],fetch_optional);
            let mut submission: Submission = serde_json::from_str(&raw.ok_or(Error::Conflict)?)?;
            submission.filing_status = status;
            submission.revision = submission.revision.checked_add(1).ok_or(Error::Invalid)?;
            let status = wire(&submission.filing_status)?;
            let raw = serde_json::to_string(&submission)?;
            execute!(
                local,
                tx,
                "UPDATE submissions SET filing_status=?,document=? WHERE id=?",
                [&status, &raw, id],
                "UPDATE submissions SET filing_status=$1,document=$2 WHERE account_id=$3 AND id=$4",
                [&status, &raw, &self.account, id]
            );
            event!(
                self,
                local,
                tx,
                submission.revision,
                "submission_filing_changed",
                id
            );
            tx.commit().await?;
            Ok(submission)
        })
    }
    pub(crate) async fn put_provider_document(&self, key: &str, document: &str) -> Result<()> {
        transaction!(self, tx, local, true, {
            execute!(local,tx,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document WHERE document<>excluded.document",[key,document],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document WHERE provider_documents.document<>excluded.document",[&self.account,key,document]);
            tx.commit().await?;
            Ok(())
        })
    }
    pub(crate) async fn provider_document(&self, key: &str) -> Result<Option<String>> {
        transaction!(self, tx, local, false, {
            Ok(scalar!(
                local,
                tx,
                "SELECT document FROM provider_documents WHERE key=?",
                [key],
                "SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2",
                [&self.account, key],
                fetch_optional
            ))
        })
    }
    pub(crate) async fn message_reference_documents(&self, message: &str) -> Result<Vec<String>> {
        let lower = format!("member-reference:{message}:");
        let upper = format!("member-reference:{message};");
        transaction!(self, tx, local, false, {
            Ok(scalar!(local,tx,"SELECT document FROM provider_documents WHERE key>=? AND key<? ORDER BY key LIMIT 256",[&lower,&upper],"SELECT document FROM provider_documents WHERE account_id=$1 AND key COLLATE \"C\">=$2 AND key COLLATE \"C\"<$3 ORDER BY key COLLATE \"C\" LIMIT 256",[&self.account,&lower,&upper],fetch_all))
        })
    }
    pub(crate) async fn claim_unsubscribe(&self, key: &str) -> Result<bool> {
        transaction!(self, tx, local, true, {
            let changed=execute!(local,tx,"INSERT INTO provider_documents(key,document) VALUES(?,'attempted') ON CONFLICT(key) DO NOTHING",[key],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,'attempted') ON CONFLICT(account_id,key) DO NOTHING",[&self.account,key]);
            tx.commit().await?;
            Ok(changed == 1)
        })
    }
    pub(crate) async fn enqueue_unsubscribe(&self, key: &str, draft: Draft) -> Result<()> {
        if draft.account_id != self.account
            || draft.to.len() != 1
            || !draft.cc.is_empty()
            || !draft.bcc.is_empty()
            || !draft.attachments.is_empty()
            || draft.in_reply_to.is_some()
            || !draft.references.is_empty()
        {
            return Err(Error::Invalid);
        }
        transaction!(self, tx, local, true, {
            let id = Uuid::new_v4().to_string();
            let record = format!("submission:{id}");
            let changed=execute!(local,tx,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO NOTHING",[key,&record],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO NOTHING",[&self.account,key,&record]);
            if changed == 0 {
                return Ok(());
            }
            let submission = Submission {
                id,
                account_id: self.account.clone(),
                operation_id: key.into(),
                draft_id: draft.id.clone(),
                draft_revision: draft.revision,
                state: SubmissionState::Scheduled,
                due_at: now(),
                timezone: "UTC".into(),
                filing_status: FilingStatus::Pending,
                revision: 1,
            };
            let raw = serde_json::to_string(&draft)?;
            let document = serde_json::to_string(&submission)?;
            execute!(local,tx,"INSERT INTO submissions(id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES(?,?,?,?,?,?,?,'scheduled',?,'pending')",[&submission.id,key,&record,&draft.id,i64::from(draft.revision),&raw,&document,&submission.due_at],"INSERT INTO submissions(account_id,id,operation_id,request,draft_id,draft_revision,snapshot,document,state,due_at,filing_status) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'scheduled',$9,'pending')",[&self.account,&submission.id,key,&record,&draft.id,i64::from(draft.revision),&raw,&document,&submission.due_at]);
            execute!(
                local,
                tx,
                "INSERT INTO outbox(kind,target_id) VALUES('submit',?)",
                [&submission.id],
                "INSERT INTO outbox(account_id,kind,target_id) VALUES($1,'submit',$2)",
                [&self.account, &submission.id]
            );
            event!(
                self,
                local,
                tx,
                1u32,
                "submission_scheduled",
                &submission.id
            );
            tx.commit().await?;
            Ok(())
        })
    }
}

#[path = "account_archive.rs"]
mod archive;

/// One native application pool is shared across account-scoped repositories.
#[derive(Clone)]
pub(crate) struct ApplicationDatabase {
    db: PgPool,
    _lease: Arc<DatabaseLease>,
}
/// Database-wide lifetime lease. Offline commands exclude every online app,
/// including apps using a different data directory or running on another host.
pub(crate) struct DatabaseLease {
    _connection: Mutex<sqlx::PgConnection>,
}
impl DatabaseLease {
    pub(crate) async fn acquire(url: &str, offline: bool) -> Result<Arc<Self>> {
        use sqlx::Connection;
        let mut connection = sqlx::PgConnection::connect(url).await?;
        let sql = if offline {
            "SELECT pg_try_advisory_lock(19217639535198209::BIGINT)"
        } else {
            "SELECT pg_try_advisory_lock_shared(19217639535198209::BIGINT)"
        };
        let locked: bool = sqlx::query_scalar(sql).fetch_one(&mut connection).await?;
        if !locked {
            return Err(Error::Conflict);
        }
        Ok(Arc::new(Self {
            _connection: Mutex::new(connection),
        }))
    }
}
impl ApplicationDatabase {
    #[cfg(test)]
    pub(crate) async fn open_postgres(url: &str) -> Result<Self> {
        Self::open_with_lease(url, DatabaseLease::acquire(url, false).await?).await
    }
    pub(crate) async fn open_with_lease(url: &str, lease: Arc<DatabaseLease>) -> Result<Self> {
        let options = sqlx::postgres::PgConnectOptions::from_str(url)?;
        let bootstrap = crate::database_deadline::postgres_pool_options(&options)
            .max_connections(1)
            .connect_with(options.clone())
            .await?;
        sqlx::query("CREATE SCHEMA IF NOT EXISTS dakia_mail")
            .execute(&bootstrap)
            .await?;
        bootstrap.close().await;
        let db = crate::database_deadline::postgres_pool_options(&options)
            .max_connections(8)
            .connect_with(options.options([("search_path", "dakia_mail,public")]))
            .await?;
        sqlx::migrate!("./migrations/account_postgres")
            .run(&db)
            .await?;
        Ok(Self { db, _lease: lease })
    }
    fn repository(&self, a: &AuthorizedAccount) -> AccountRepository {
        AccountRepository {
            backend: Backend::Postgres(self.db.clone()),
            account: a.id.clone(),
            owner: a.user_id.clone(),
        }
    }
}

impl AccountRepository {
    pub(crate) async fn defer_submission(
        &self,
        id: &str,
        from: SubmissionState,
        delay: u64,
    ) -> Result<Submission> {
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE submissions SET state=state WHERE id=?",
                [id],
                "UPDATE submissions SET state=state WHERE account_id=$1 AND id=$2",
                [&self.account, id]
            );
            let raw: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM submissions WHERE id=?",
                [id],
                "SELECT document FROM submissions WHERE account_id=$1 AND id=$2",
                [&self.account, id],
                fetch_optional
            );
            let mut submission: Submission = serde_json::from_str(&raw.ok_or(Error::NotFound)?)?;
            if submission.state != from {
                return Err(Error::Conflict);
            }
            let key = format!("send:attempts:{id}");
            let attempts: Option<String> = scalar!(
                local,
                tx,
                "SELECT document FROM provider_documents WHERE key=?",
                [&key],
                "SELECT document FROM provider_documents WHERE account_id=$1 AND key=$2",
                [&self.account, &key],
                fetch_optional
            );
            let attempts = attempts
                .map(|s| s.parse::<u32>().map_err(|_| Error::Invalid))
                .transpose()?
                .unwrap_or(0);
            submission.state = if attempts >= 5 {
                SubmissionState::Failed
            } else {
                SubmissionState::Scheduled
            };
            submission.due_at = (Utc::now()
                + Duration::seconds(
                    delay.clamp(1, 86400).max(5 * (1u64 << attempts.min(10))) as i64
                ))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
            submission.revision = submission.revision.checked_add(1).ok_or(Error::Invalid)?;
            let state = wire(&submission.state)?;
            let raw = serde_json::to_string(&submission)?;
            execute!(local,tx,"UPDATE submissions SET state=?,due_at=?,lease_until=NULL,document=? WHERE id=?",[&state,&submission.due_at,&raw,id],"UPDATE submissions SET state=$1,due_at=$2,lease_until=NULL,document=$3 WHERE account_id=$4 AND id=$5",[&state,&submission.due_at,&raw,&self.account,id]);
            let attempts = (attempts + 1).to_string();
            execute!(local,tx,"INSERT INTO provider_documents(key,document) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET document=excluded.document",[&key,&attempts],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,$2,$3) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&self.account,&key,&attempts]);
            event!(
                self,
                local,
                tx,
                submission.revision,
                "submission_changed",
                id
            );
            tx.commit().await?;
            Ok(submission)
        })
    }
    pub(crate) async fn uncertain_submissions(&self) -> Result<Vec<Submission>> {
        let cutoff = Utc::now().timestamp() - 60;
        transaction!(self, tx, local, false, {
            let rows:Vec<String>=scalar!(local,tx,"SELECT s.document FROM submissions s WHERE s.state='delivery_unknown' AND COALESCE((SELECT CAST(document AS INTEGER) FROM provider_documents WHERE key='send:lookup:'||s.id),0)<=? ORDER BY COALESCE((SELECT CAST(document AS INTEGER) FROM provider_documents WHERE key='send:lookup:'||s.id),0),due_at LIMIT 10",[cutoff],"SELECT s.document FROM submissions s WHERE s.account_id=$1 AND s.state='delivery_unknown' AND COALESCE((SELECT document::BIGINT FROM provider_documents WHERE account_id=s.account_id AND key='send:lookup:'||s.id),0)<=$2 ORDER BY COALESCE((SELECT document::BIGINT FROM provider_documents WHERE account_id=s.account_id AND key='send:lookup:'||s.id),0),due_at LIMIT 10",[&self.account,cutoff],fetch_all);
            rows.into_iter()
                .map(|raw| Ok(serde_json::from_str(&raw)?))
                .collect()
        })
    }
    pub(crate) async fn confirm_delivery(&self, id: &str) -> Result<Submission> {
        transaction!(self, tx, local, true, {
            execute!(
                local,
                tx,
                "UPDATE submissions SET state=state WHERE id=?",
                [id],
                "UPDATE submissions SET state=state WHERE account_id=$1 AND id=$2",
                [&self.account, id]
            );
            let raw:Option<String>=scalar!(local,tx,"SELECT document FROM submissions WHERE id=? AND state='delivery_unknown'",[id],"SELECT document FROM submissions WHERE account_id=$1 AND id=$2 AND state='delivery_unknown'",[&self.account,id],fetch_optional);
            let mut submission: Submission = serde_json::from_str(&raw.ok_or(Error::Conflict)?)?;
            submission.state = SubmissionState::Accepted;
            submission.revision = submission.revision.checked_add(1).ok_or(Error::Invalid)?;
            let raw = serde_json::to_string(&submission)?;
            execute!(
                local,
                tx,
                "UPDATE submissions SET state='accepted',document=? WHERE id=?",
                [&raw, id],
                "UPDATE submissions SET state='accepted',document=$1 WHERE account_id=$2 AND id=$3",
                [&raw, &self.account, id]
            );
            event!(
                self,
                local,
                tx,
                submission.revision,
                "submission_changed",
                id
            );
            tx.commit().await?;
            Ok(submission)
        })
    }
}
