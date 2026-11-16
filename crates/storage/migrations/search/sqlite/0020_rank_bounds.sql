-- Compact, body-free ranking bounds and direct per-message field checks.
-- These are derived access paths. Retained text/vectors and stage receipts
-- stay valid; neither an index rebuild nor re-embedding is required.
CREATE INDEX lexical_bound_lookup ON lexical_chunks(account_id,id,message_id,revision,field,ordinal);
CREATE INDEX lexical_field_lookup ON lexical_chunks(account_id,message_id,field,content_version,revision,ordinal);
CREATE INDEX documents_subject_identity ON documents(account_id,subject,message_id) WHERE state<>'deleted';
