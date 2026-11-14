CREATE TABLE received_cache (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 message_id TEXT NOT NULL,blob_id TEXT NOT NULL,size_bytes BIGINT NOT NULL,last_access TEXT NOT NULL,
 PRIMARY KEY(account_id,message_id),UNIQUE(account_id,blob_id),
 FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id)
);
CREATE TABLE recipient_evidence (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 source_id TEXT NOT NULL,email TEXT NOT NULL,document TEXT NOT NULL,observed_at TEXT NOT NULL,
 email_key TEXT GENERATED ALWAYS AS (translate(email,'ABCDEFGHIJKLMNOPQRSTUVWXYZ','abcdefghijklmnopqrstuvwxyz')) STORED,
 PRIMARY KEY(account_id,source_id,email_key)
);
CREATE INDEX recipients_email ON recipient_evidence(account_id,email_key);
