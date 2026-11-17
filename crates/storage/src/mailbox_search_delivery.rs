//! Application-side outbox delivery. No transaction crosses the search-store call.
use super::*;
type DeliveryRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
);
use crate::search_backend::Document;
pub(crate) type SearchDelivery = (String, i64, Option<Document>);
impl MailboxTransaction<'_> {
    async fn requeue_search_index(&mut self, identity: &str) -> Result<()> {
        write!(
            self,
            "UPDATE search_index_clock SET value=value+1 WHERE id=1",
            [],
            "UPDATE account_locks SET search_sequence=search_sequence+1 WHERE account_id=$1",
            [&self.account]
        );
        write!(self,"UPDATE search_documents SET body_state='pending',retry_at=0,delivered=0 WHERE body_payload=0",[],"UPDATE search_documents SET body_state='pending',retry_at=0,delivered=0 WHERE account_id=$1 AND body_payload=0",[&self.account]);
        write!(self,"INSERT INTO search_index_outbox(message_id,sequence) SELECT d.message_id,c.value FROM search_documents d CROSS JOIN search_index_clock c WHERE c.id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence",[],"INSERT INTO search_index_outbox(account_id,message_id,sequence) SELECT d.account_id,d.message_id,c.search_sequence FROM search_documents d JOIN account_locks c ON c.account_id=d.account_id WHERE d.account_id=$1 ON CONFLICT(account_id,message_id) DO UPDATE SET sequence=excluded.sequence",[&self.account]);
        write!(self,"INSERT INTO provider_documents(key,document) VALUES('search:database',?) ON CONFLICT(key) DO UPDATE SET document=excluded.document",[identity],"INSERT INTO provider_documents(account_id,key,document) VALUES($1,'search:database',$2) ON CONFLICT(account_id,key) DO UPDATE SET document=excluded.document",[&self.account,identity]);
        Ok(())
    }
    async fn search_deliveries(&mut self) -> Result<Vec<SearchDelivery>> {
        let deliveries:Vec<(String,i64)>=tuples!(self,"SELECT message_id,sequence FROM search_index_outbox ORDER BY sequence,message_id LIMIT 200",[],"SELECT message_id,sequence FROM search_index_outbox WHERE account_id=$1 ORDER BY sequence,message_id COLLATE \"C\" LIMIT 200",[&self.account],fetch_all);
        let mut records = Vec::new();
        let mut bytes = 0;
        for (id, sequence) in deliveries {
            // Keep dirty same-source records pending instead of replacing a
            // retained destination body with a transient metadata tombstone.
            let preserve:bool=value!(self,"SELECT EXISTS(SELECT 1 FROM messages m JOIN search_documents d ON d.message_id=m.id JOIN search_eligible_messages e ON e.id=m.id JOIN search_dirty dirty ON dirty.message_id=m.id WHERE m.id=? AND d.source=json_extract(m.document,'$.provider_id'))",[&id],"SELECT EXISTS(SELECT 1 FROM messages m JOIN search_documents d ON d.account_id=m.account_id AND d.message_id=m.id JOIN search_eligible_messages e ON e.account_id=m.account_id AND e.id=m.id JOIN search_dirty dirty ON dirty.account_id=m.account_id AND dirty.message_id=m.id WHERE m.account_id=$1 AND m.id=$2 AND d.source=m.source)",[&self.account,&id],fetch_one);
            if preserve {
                continue;
            }
            let row:Option<DeliveryRow>=tuples!(self,"SELECT m.document,d.source,d.sender,d.recipients,d.subject,d.body,d.participants,d.mime_recipients,d.attachment_text,d.body_state,d.body_payload,d.attachment_details,(SELECT until FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id')) FROM search_documents d JOIN messages m ON m.id=d.message_id JOIN search_eligible_messages e ON e.id=m.id WHERE d.message_id=? AND d.source=json_extract(m.document,'$.provider_id') AND NOT EXISTS(SELECT 1 FROM search_dirty WHERE message_id=d.message_id)",[&id],"SELECT m.document,d.source,d.sender,d.recipients,d.subject,d.body,d.participants,d.mime_recipients,d.attachment_text,d.body_state,d.body_payload,d.attachment_details,(SELECT until FROM snoozes s WHERE s.account_id=m.account_id AND s.thread_id=m.thread_id) FROM search_documents d JOIN messages m ON m.account_id=d.account_id AND m.id=d.message_id JOIN search_eligible_messages e ON e.account_id=m.account_id AND e.id=m.id WHERE d.account_id=$1 AND d.message_id=$2 AND d.source=m.source AND NOT EXISTS(SELECT 1 FROM search_dirty WHERE account_id=d.account_id AND message_id=d.message_id)",[&self.account,&id],fetch_optional);
            let document = row
                .map(
                    |(
                        raw,
                        source,
                        sender,
                        recipients,
                        subject,
                        body,
                        participants,
                        mime_recipients,
                        attachments,
                        state,
                        body_payload,
                        attachment_details,
                        snoozed_until,
                    )|
                     -> Result<Document> {
                        Ok(Document {
                            message: serde_json::from_str(&raw)?,
                            source,
                            sender,
                            recipients,
                            subject,
                            body,
                            participants,
                            mime_recipients,
                            attachments,
                            state,
                            body_payload: body_payload == 1,
                            attachment_details,
                            snoozed_until,
                        })
                    },
                )
                .transpose()?;
            let size = document
                .as_ref()
                .map(|d| d.body.len() + d.attachments.len() + d.participants.len())
                .unwrap_or(0);
            if !records.is_empty() && bytes + size > 8 * 1024 * 1024 {
                break;
            }
            bytes += size;
            records.push((id, sequence, document));
        }
        Ok(records)
    }
    async fn acknowledge_search_deliveries(&mut self, records: &[SearchDelivery]) -> Result<()> {
        for (id, sequence, document) in records {
            write!(self,"DELETE FROM search_index_outbox WHERE message_id=? AND sequence=?",[id,*sequence],"DELETE FROM search_index_outbox WHERE account_id=$1 AND message_id=$2 AND sequence=$3",[&self.account,id,*sequence]);
            if document.is_some() {
                write!(self,"UPDATE search_documents SET delivered=1,delivered_version=? WHERE message_id=? AND delivered_version<=? AND NOT EXISTS(SELECT 1 FROM search_index_outbox WHERE message_id=?)",[*sequence,id,*sequence,id],"UPDATE search_documents d SET delivered=1,delivered_version=$3 WHERE d.account_id=$1 AND d.message_id=$2 AND d.delivered_version<=$3 AND NOT EXISTS(SELECT 1 FROM search_index_outbox WHERE account_id=d.account_id AND message_id=d.message_id)",[&self.account,id,*sequence]);
                write!(self,"UPDATE search_documents SET body_payload=0,body='',participants='',mime_recipients='',attachment_text='' WHERE message_id=? AND body_payload=1 AND NOT EXISTS(SELECT 1 FROM search_index_outbox WHERE message_id=?)",[id,id],"UPDATE search_documents d SET body_payload=0,body='',participants='',mime_recipients='',attachment_text='' WHERE d.account_id=$1 AND d.message_id=$2 AND d.body_payload=1 AND NOT EXISTS(SELECT 1 FROM search_index_outbox WHERE account_id=d.account_id AND message_id=d.message_id)",[&self.account,id]);
            }
        }
        Ok(())
    }
}
impl AccountRepository {
    pub(crate) async fn requeue_search_index(&self, identity: &str) -> Result<()> {
        scoped!(self, scope, true, {
            scope.requeue_search_index(identity).await
        })
    }
    pub(crate) async fn search_deliveries(&self) -> Result<Vec<SearchDelivery>> {
        scoped!(self, scope, false, { scope.search_deliveries().await })
    }
    pub(crate) async fn acknowledge_search_deliveries(
        &self,
        records: &[SearchDelivery],
    ) -> Result<()> {
        scoped!(self, scope, true, {
            scope.acknowledge_search_deliveries(records).await
        })
    }
}
