CREATE TABLE message_identities (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 provider_id TEXT NOT NULL,message_id TEXT NOT NULL,PRIMARY KEY(account_id,provider_id)
);
CREATE INDEX message_identities_message ON message_identities(account_id,message_id);
