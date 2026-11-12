CREATE TABLE message_classifications (
 message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED, automatic TEXT, kind_override TEXT,
 added_types TEXT NOT NULL DEFAULT '[]', removed_types TEXT NOT NULL DEFAULT '[]',
 attention_override INTEGER, requested INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE classification_senders (sender TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('personal','automated')));
-- Retain legacy manual values separately. Fine-grained types are detected later.
INSERT INTO message_classifications(message_id,kind_override)
SELECT id,CASE WHEN category_override IS NULL THEN NULL WHEN category_override IN ('people','other') THEN 'personal' ELSE 'automated' END FROM messages;
UPDATE messages SET document=json_set(document,'$.classification',json_object(
 'kind',CASE WHEN category_override IN ('transactions','notifications','newsletters') THEN 'automated' ELSE 'personal' END,
 'status','pending','types',json('[]'),'details',json('[]'),'attention',json('[]'),'attention_override',NULL,
 'source',CASE WHEN category_override IS NULL THEN 'automatic' ELSE 'message' END,'policy_revision',''));
DROP INDEX messages_unread_classification;
CREATE INDEX messages_pending_classification ON messages(json_extract(document,'$.received_at') DESC,id)
WHERE json_extract(document,'$.classification.status')='pending';
CREATE INDEX messages_inbox_kind ON messages(json_extract(document,'$.classification.kind'),json_extract(document,'$.received_at') DESC,id DESC);
CREATE INDEX classifications_requested ON message_classifications(requested,message_id);
-- Suppress outstanding pre-upgrade notifications before historical reprocessing.
UPDATE push_jobs SET state='skipped' WHERE state IN ('pending','running');
-- Historical reprocessing must never reopen completed notification receipts.
UPDATE model_jobs SET state='cancelled',document=json_set(document,'$.status','cancelled') WHERE state IN ('pending','running') AND json_extract(document,'$.kind')='classify';

CREATE INDEX messages_classification_sender ON messages(lower(trim(json_extract(document,'$.sender.email'))),id);
