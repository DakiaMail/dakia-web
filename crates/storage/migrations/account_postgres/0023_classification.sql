CREATE TABLE message_classifications (
 account_id TEXT NOT NULL, message_id TEXT NOT NULL, automatic TEXT, kind_override TEXT,
 added_types TEXT NOT NULL DEFAULT '[]', removed_types TEXT NOT NULL DEFAULT '[]',
 attention_override BOOLEAN, requested BOOLEAN NOT NULL DEFAULT FALSE,
 PRIMARY KEY(account_id,message_id), FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE classification_senders (account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE, sender TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('personal','automated')), PRIMARY KEY(account_id,sender));
INSERT INTO message_classifications(account_id,message_id,kind_override) SELECT account_id,id,CASE WHEN category_override IS NULL THEN NULL WHEN category_override IN ('people','other') THEN 'personal' ELSE 'automated' END FROM messages;
UPDATE messages SET document=jsonb_set(document::jsonb,'{classification}',jsonb_build_object('kind',CASE WHEN category_override IN ('transactions','notifications','newsletters') THEN 'automated' ELSE 'personal' END,'status','pending','types','[]'::jsonb,'details','[]'::jsonb,'attention','[]'::jsonb,'attention_override',NULL,'source',CASE WHEN category_override IS NULL THEN 'automatic' ELSE 'message' END,'policy_revision',''))::TEXT;
CREATE INDEX messages_pending_classification ON messages(account_id,received_at DESC,id) WHERE document::jsonb#>>'{classification,status}'='pending';
CREATE INDEX messages_inbox_kind ON messages(account_id,(document::jsonb#>>'{classification,kind}'),received_at DESC,id DESC);
CREATE INDEX classifications_requested ON message_classifications(account_id,requested,message_id);
CREATE INDEX messages_classification_sender ON messages(account_id,lower(trim(document::jsonb#>>'{sender,email}')),id);
UPDATE push_jobs SET state='skipped' WHERE state IN ('pending','running');
UPDATE model_jobs SET state='cancelled',document=jsonb_set(document::jsonb,'{status}','"cancelled"')::TEXT WHERE state IN ('pending','running') AND kind='classify';
