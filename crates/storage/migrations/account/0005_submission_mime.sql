CREATE TABLE submission_mime (submission_id TEXT PRIMARY KEY REFERENCES submissions(id), raw BLOB NOT NULL, recipients TEXT NOT NULL, message_id TEXT NOT NULL);
