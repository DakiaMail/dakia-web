CREATE INDEX lexical_field_lookup ON lexical_chunks(account_id,message_id,field,content_version,revision,ordinal);
-- Hash only narrows candidates; SQL still checks the complete subject. This
-- avoids putting an arbitrarily long header into a PostgreSQL B-tree key.
CREATE INDEX documents_subject_identity ON documents(account_id,md5(subject)) WHERE state<>'deleted';
