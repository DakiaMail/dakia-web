CREATE INDEX messages_search_thread ON messages(json_extract(document,'$.thread_id'));
CREATE TRIGGER search_snooze_insert AFTER INSERT ON snoozes BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT m.id,c.value FROM messages m CROSS JOIN search_index_clock c WHERE json_extract(m.document,'$.thread_id')=NEW.thread_id AND c.id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
CREATE TRIGGER search_snooze_update AFTER UPDATE OF until ON snoozes BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT m.id,c.value FROM messages m CROSS JOIN search_index_clock c WHERE json_extract(m.document,'$.thread_id')=NEW.thread_id AND c.id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
CREATE TRIGGER search_snooze_delete AFTER DELETE ON snoozes BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT m.id,c.value FROM messages m CROSS JOIN search_index_clock c WHERE json_extract(m.document,'$.thread_id')=OLD.thread_id AND c.id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
UPDATE search_index_clock SET value=value+1 WHERE id=1;
INSERT INTO search_index_outbox(message_id,sequence) SELECT m.id,c.value FROM messages m CROSS JOIN search_index_clock c WHERE c.id=1 AND EXISTS(SELECT 1 FROM snoozes s WHERE s.thread_id=json_extract(m.document,'$.thread_id')) ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
