-- Retain earlier installations' durable compatibility rows during backend moves,
-- even when current workers use the newer provider/recipient representations.
CREATE TABLE sync_cursors (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 folder_id TEXT NOT NULL,cursor TEXT NOT NULL,initial_boundary TEXT,
 PRIMARY KEY(account_id,folder_id)
);
CREATE TABLE provider_refs (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,provider_key TEXT NOT NULL,message_id TEXT NOT NULL,
 folder_id TEXT NOT NULL,uidvalidity BIGINT NOT NULL,uid BIGINT NOT NULL,
 PRIMARY KEY(account_id,provider_key),UNIQUE(account_id,folder_id,uidvalidity,uid),
 FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id)
);
CREATE TABLE push_subscriptions (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 id TEXT NOT NULL,document TEXT NOT NULL,PRIMARY KEY(account_id,id)
);
CREATE TABLE recipients (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 email TEXT NOT NULL,name TEXT,evidence TEXT NOT NULL CHECK(evidence IN ('sent','deliberate')),
 score BIGINT NOT NULL,PRIMARY KEY(account_id,email)
);
