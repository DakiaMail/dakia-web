CREATE TABLE folder_scans (folder TEXT PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 1, continuation TEXT, completed_at INTEGER);
CREATE TABLE folder_memberships (folder TEXT NOT NULL, message_id TEXT NOT NULL, generation INTEGER NOT NULL, PRIMARY KEY(folder,message_id));
