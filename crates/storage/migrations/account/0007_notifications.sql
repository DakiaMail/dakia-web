ALTER TABLE folder_scans ADD COLUMN initial_boundary INTEGER;
CREATE TABLE push_jobs (message_id TEXT NOT NULL, subscription_id TEXT NOT NULL, document TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending', next_attempt INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(message_id,subscription_id));
