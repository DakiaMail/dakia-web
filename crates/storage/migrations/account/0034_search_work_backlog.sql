-- Count a message once across acquisition, delivery and embedding stages.
-- Only ineligible cleanup lies outside eligible minus completed messages.
CREATE TABLE search_cleanup_work(message_id TEXT PRIMARY KEY,dirty INTEGER NOT NULL,outbox INTEGER NOT NULL,eligible INTEGER NOT NULL);
CREATE TABLE search_cleanup_totals(id INTEGER PRIMARY KEY CHECK(id=1),pending INTEGER NOT NULL DEFAULT 0);
INSERT INTO search_cleanup_totals VALUES(1,0);
CREATE TRIGGER search_cleanup_insert AFTER INSERT ON search_cleanup_work BEGIN UPDATE search_cleanup_totals SET pending=pending+(NOT NEW.eligible AND (NEW.dirty OR NEW.outbox)) WHERE id=1;END;
CREATE TRIGGER search_cleanup_delete AFTER DELETE ON search_cleanup_work BEGIN UPDATE search_cleanup_totals SET pending=pending-(NOT OLD.eligible AND (OLD.dirty OR OLD.outbox)) WHERE id=1;END;
CREATE TRIGGER search_cleanup_update AFTER UPDATE ON search_cleanup_work BEGIN UPDATE search_cleanup_totals SET pending=pending+(NOT NEW.eligible AND (NEW.dirty OR NEW.outbox))-(NOT OLD.eligible AND (OLD.dirty OR OLD.outbox)) WHERE id=1;END;
INSERT INTO search_cleanup_work SELECT id,EXISTS(SELECT 1 FROM search_dirty WHERE message_id=id),EXISTS(SELECT 1 FROM search_index_outbox WHERE message_id=id),EXISTS(SELECT 1 FROM search_eligible_messages WHERE search_eligible_messages.id=p.id) FROM (SELECT message_id AS id FROM search_dirty UNION SELECT message_id FROM search_index_outbox) p;
CREATE TRIGGER search_cleanup_dirty_insert AFTER INSERT ON search_dirty BEGIN
 INSERT INTO search_cleanup_work VALUES(NEW.message_id,1,EXISTS(SELECT 1 FROM search_index_outbox WHERE message_id=NEW.message_id),EXISTS(SELECT 1 FROM search_eligible_messages WHERE id=NEW.message_id)) ON CONFLICT(message_id) DO UPDATE SET dirty=1,outbox=excluded.outbox,eligible=excluded.eligible;
END;
CREATE TRIGGER search_cleanup_outbox_insert AFTER INSERT ON search_index_outbox BEGIN
 INSERT INTO search_cleanup_work VALUES(NEW.message_id,EXISTS(SELECT 1 FROM search_dirty WHERE message_id=NEW.message_id),1,EXISTS(SELECT 1 FROM search_eligible_messages WHERE id=NEW.message_id)) ON CONFLICT(message_id) DO UPDATE SET dirty=excluded.dirty,outbox=1,eligible=excluded.eligible;
END;
CREATE TRIGGER search_cleanup_outbox_update AFTER UPDATE ON search_index_outbox BEGIN
 UPDATE search_cleanup_work SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages WHERE id=NEW.message_id) WHERE message_id=NEW.message_id;
END;
CREATE TRIGGER search_cleanup_dirty_delete AFTER DELETE ON search_dirty BEGIN
 UPDATE search_cleanup_work SET dirty=0,eligible=EXISTS(SELECT 1 FROM search_eligible_messages WHERE id=OLD.message_id) WHERE message_id=OLD.message_id;
 DELETE FROM search_cleanup_work WHERE message_id=OLD.message_id AND dirty=0 AND outbox=0;
END;
CREATE TRIGGER search_cleanup_outbox_delete AFTER DELETE ON search_index_outbox BEGIN
 UPDATE search_cleanup_work SET outbox=0,eligible=EXISTS(SELECT 1 FROM search_eligible_messages WHERE id=OLD.message_id) WHERE message_id=OLD.message_id;
 DELETE FROM search_cleanup_work WHERE message_id=OLD.message_id AND dirty=0 AND outbox=0;
END;
CREATE TRIGGER search_cleanup_message_insert AFTER INSERT ON messages BEGIN UPDATE search_cleanup_work SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages WHERE id=NEW.id) WHERE message_id=NEW.id;END;
CREATE TRIGGER search_cleanup_message_update AFTER UPDATE OF document ON messages BEGIN UPDATE search_cleanup_work SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages WHERE id=NEW.id) WHERE message_id=NEW.id;END;
CREATE TRIGGER search_cleanup_message_delete AFTER DELETE ON messages BEGIN UPDATE search_cleanup_work SET eligible=0 WHERE message_id=OLD.id;END;
CREATE TRIGGER search_cleanup_epoch_insert AFTER INSERT ON provider_documents WHEN NEW.key GLOB 'thread:epoch:*' BEGIN
 UPDATE search_cleanup_work SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.id=search_cleanup_work.message_id) WHERE message_id IN (SELECT id FROM messages WHERE CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN json_extract(json_extract(document,'$.provider_id'),'$.folder') END=substr(NEW.key,14));
END;
CREATE TRIGGER search_cleanup_epoch_update AFTER UPDATE OF document ON provider_documents WHEN NEW.key GLOB 'thread:epoch:*' BEGIN
 UPDATE search_cleanup_work SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.id=search_cleanup_work.message_id) WHERE message_id IN (SELECT id FROM messages WHERE CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN json_extract(json_extract(document,'$.provider_id'),'$.folder') END=substr(NEW.key,14));
END;
CREATE TRIGGER search_cleanup_epoch_delete AFTER DELETE ON provider_documents WHEN OLD.key GLOB 'thread:epoch:*' BEGIN
 UPDATE search_cleanup_work SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.id=search_cleanup_work.message_id) WHERE message_id IN (SELECT id FROM messages WHERE CASE WHEN json_valid(json_extract(document,'$.provider_id')) THEN json_extract(json_extract(document,'$.provider_id'),'$.folder') END=substr(OLD.key,14));
END;
