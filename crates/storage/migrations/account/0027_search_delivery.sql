-- Not FK-bound: deletion receipts must survive removal of the message itself.
CREATE TABLE search_index_clock (id INTEGER PRIMARY KEY CHECK(id=1),value INTEGER NOT NULL);
INSERT INTO search_index_clock VALUES(1,0);
CREATE TABLE search_index_outbox (message_id TEXT PRIMARY KEY,sequence INTEGER NOT NULL);
CREATE INDEX search_index_outbox_order ON search_index_outbox(sequence);
CREATE TRIGGER search_delivery_insert AFTER INSERT ON search_documents BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT new.message_id,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
CREATE TRIGGER search_delivery_update AFTER UPDATE ON search_documents BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT new.message_id,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
CREATE TRIGGER search_delivery_delete AFTER DELETE ON search_documents BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT old.message_id,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
CREATE TRIGGER search_delivery_metadata AFTER UPDATE OF document ON messages BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT new.id,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
INSERT INTO search_index_outbox(message_id,sequence) SELECT message_id,0 FROM search_documents;
ALTER TABLE search_totals ADD COLUMN delivery_pending INTEGER NOT NULL DEFAULT 0;
UPDATE search_totals SET delivery_pending=(SELECT count(*) FROM search_index_outbox);
CREATE TRIGGER search_delivery_count_insert AFTER INSERT ON search_index_outbox BEGIN UPDATE search_totals SET delivery_pending=delivery_pending+1 WHERE id=1; END;
CREATE TRIGGER search_delivery_count_delete AFTER DELETE ON search_index_outbox BEGIN UPDATE search_totals SET delivery_pending=delivery_pending-1 WHERE id=1; END;
