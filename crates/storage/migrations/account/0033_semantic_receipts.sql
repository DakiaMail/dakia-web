ALTER TABLE search_documents ADD COLUMN delivered_version INTEGER NOT NULL DEFAULT 0;
CREATE INDEX search_delivery_upgrade ON search_documents(delivered_version,message_id);
CREATE TABLE search_semantic_profile(id INTEGER PRIMARY KEY CHECK(id=1),model_revision TEXT NOT NULL,available INTEGER NOT NULL);
CREATE TABLE search_semantic_totals(model_revision TEXT PRIMARY KEY,processed INTEGER NOT NULL DEFAULT 0);
CREATE TABLE search_semantic_receipts(message_id TEXT PRIMARY KEY REFERENCES search_documents(message_id) ON DELETE CASCADE,version INTEGER NOT NULL,source TEXT NOT NULL,model_revision TEXT NOT NULL);
CREATE TRIGGER search_semantic_insert AFTER INSERT ON search_semantic_receipts BEGIN
 INSERT INTO search_semantic_totals(model_revision,processed) VALUES(NEW.model_revision,1) ON CONFLICT(model_revision) DO UPDATE SET processed=processed+1;
END;
CREATE TRIGGER search_semantic_delete AFTER DELETE ON search_semantic_receipts BEGIN
 UPDATE search_semantic_totals SET processed=processed-1 WHERE model_revision=OLD.model_revision;
END;
CREATE TRIGGER search_semantic_update AFTER UPDATE ON search_semantic_receipts BEGIN
 UPDATE search_semantic_totals SET processed=processed-1 WHERE model_revision=OLD.model_revision;
 INSERT INTO search_semantic_totals(model_revision,processed) VALUES(NEW.model_revision,1) ON CONFLICT(model_revision) DO UPDATE SET processed=processed+1;
END;
-- Invalidate completion in the same transaction as any mailbox publication,
-- including metadata-only changes. The target reissues a versioned receipt
-- without repeating inference when its retained vectors are still valid.
CREATE TRIGGER search_semantic_dirty AFTER INSERT ON search_dirty BEGIN DELETE FROM search_semantic_receipts WHERE message_id=NEW.message_id;END;
CREATE TRIGGER search_semantic_outbox_insert AFTER INSERT ON search_index_outbox BEGIN DELETE FROM search_semantic_receipts WHERE message_id=NEW.message_id;END;
CREATE TRIGGER search_semantic_outbox_update AFTER UPDATE ON search_index_outbox BEGIN DELETE FROM search_semantic_receipts WHERE message_id=NEW.message_id;END;
