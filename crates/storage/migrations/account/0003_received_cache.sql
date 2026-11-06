CREATE TABLE received_cache (message_id TEXT PRIMARY KEY REFERENCES messages(id), blob_id TEXT NOT NULL UNIQUE, size_bytes INTEGER NOT NULL, last_access TEXT NOT NULL);
CREATE TABLE message_identities (provider_id TEXT PRIMARY KEY, message_id TEXT NOT NULL UNIQUE);
