-- Effective classifications change transactionally with message documents.
CREATE INDEX messages_inbox_effective_kind ON messages(COALESCE(json_extract(document,'$.classification.kind'),'personal'),json_extract(document,'$.received_at') DESC,id DESC)
WHERE instr(upper(json_extract(document,'$.folder_ids')),'"INBOX"')>0;
CREATE TABLE message_types(message_id TEXT NOT NULL,email_type TEXT NOT NULL,received_at TEXT NOT NULL,PRIMARY KEY(message_id,email_type));
CREATE INDEX message_types_order ON message_types(email_type,received_at DESC,message_id DESC);
INSERT OR IGNORE INTO message_types SELECT m.id,t.value,json_extract(m.document,'$.received_at') FROM messages m,json_each(m.document,'$.classification.types') t;
CREATE TRIGGER message_types_insert AFTER INSERT ON messages BEGIN
 INSERT OR IGNORE INTO message_types SELECT NEW.id,t.value,json_extract(NEW.document,'$.received_at') FROM json_each(NEW.document,'$.classification.types') t;
END;
CREATE TRIGGER message_types_update AFTER UPDATE OF document ON messages
WHEN json_extract(OLD.document,'$.classification.types') IS NOT json_extract(NEW.document,'$.classification.types')
 OR json_extract(OLD.document,'$.received_at') IS NOT json_extract(NEW.document,'$.received_at')
BEGIN
 DELETE FROM message_types WHERE message_id=OLD.id;
 INSERT OR IGNORE INTO message_types SELECT NEW.id,t.value,json_extract(NEW.document,'$.received_at') FROM json_each(NEW.document,'$.classification.types') t;
END;
CREATE TRIGGER message_types_delete AFTER DELETE ON messages BEGIN
 DELETE FROM message_types WHERE message_id=OLD.id;
END;
