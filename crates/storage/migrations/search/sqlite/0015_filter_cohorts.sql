-- Keep bounded filter-cohort discovery on compact metadata and chunk keys.
-- Reading full document/MIME text and embedding pages here needlessly turns
-- an indexed participant filter into thousands of random payload reads.
CREATE INDEX documents_filter_cohort ON documents(
 rowid,account_id,message_id,source,content_version,state,sender,recipients,
 subject,received_date,unread,starred,category,snoozed_until
);
CREATE INDEX chunks_filter_cohort ON chunks(
 account_id,message_id,model_revision,source,content_version,id
);
