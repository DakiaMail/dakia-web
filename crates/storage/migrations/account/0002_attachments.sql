CREATE TABLE attachment_metadata (id TEXT PRIMARY KEY REFERENCES blobs(id), document TEXT NOT NULL);
CREATE TABLE provider_refs (provider_key TEXT PRIMARY KEY, message_id TEXT NOT NULL REFERENCES messages(id), folder_id TEXT NOT NULL, uidvalidity INTEGER NOT NULL, uid INTEGER NOT NULL, UNIQUE(folder_id,uidvalidity,uid));
