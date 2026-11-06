-- IMAP MOVE changes folder/UID but must retain the local message and owner state.
-- A provider reference is unique; multiple historical references may name one message.
CREATE TABLE message_identity_aliases (provider_id TEXT PRIMARY KEY, message_id TEXT NOT NULL);
INSERT INTO message_identity_aliases(provider_id, message_id) SELECT provider_id, message_id FROM message_identities;
DROP TABLE message_identities;
ALTER TABLE message_identity_aliases RENAME TO message_identities;
CREATE INDEX message_identities_message ON message_identities(message_id);
