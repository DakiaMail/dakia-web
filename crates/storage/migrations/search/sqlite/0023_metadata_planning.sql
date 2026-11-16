-- Canonical metadata planning stays independent of secondary FTS postings.
-- These covering indexes never duplicate body or attachment payloads.
CREATE INDEX documents_sender_identity ON documents(account_id,sender,state);
CREATE INDEX documents_recipients_identity ON documents(account_id,recipients,state);
