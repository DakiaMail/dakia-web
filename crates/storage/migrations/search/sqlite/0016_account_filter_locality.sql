-- Scope rowid probes by account before reading compact filter metadata. Global
-- trigram postings may contain other accounts; their rows need no payload read.
DROP INDEX documents_filter_cohort;
CREATE INDEX documents_filter_cohort ON documents(
 account_id,rowid,message_id,source,content_version,state,sender,recipients,
 subject,received_date,unread,starred,category,snoozed_until
);
