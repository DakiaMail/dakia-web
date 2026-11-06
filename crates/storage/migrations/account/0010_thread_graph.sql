-- Header evidence stays in the existing private per-account database. This
-- inverted index is derived, not a second source of message/membership truth.
ALTER TABLE folder_scans ADD COLUMN coverage_limited INTEGER NOT NULL DEFAULT 0;
ALTER TABLE folder_scans ADD COLUMN uidvalidity INTEGER;
CREATE TABLE thread_generation (id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL);
INSERT INTO thread_generation VALUES(1,0);
CREATE TABLE thread_verifications (component TEXT PRIMARY KEY, revision INTEGER NOT NULL, checked_at INTEGER NOT NULL);
CREATE TRIGGER thread_message_insert AFTER INSERT ON messages BEGIN UPDATE thread_generation SET revision=revision+1; END;
CREATE TRIGGER thread_message_update AFTER UPDATE OF document ON messages WHEN OLD.document<>NEW.document BEGIN UPDATE thread_generation SET revision=revision+1; END;
CREATE TRIGGER thread_message_delete AFTER DELETE ON messages BEGIN UPDATE thread_generation SET revision=revision+1; END;
CREATE TRIGGER thread_topology_update AFTER UPDATE OF document ON provider_documents
WHEN OLD.document<>NEW.document AND (NEW.key='probe' OR NEW.key='thread:folders' OR substr(NEW.key,1,13)='thread:epoch:')
BEGIN UPDATE thread_generation SET revision=revision+1; END;
CREATE TRIGGER thread_topology_insert AFTER INSERT ON provider_documents
WHEN NEW.key='probe' OR NEW.key='thread:folders' OR substr(NEW.key,1,13)='thread:epoch:'
BEGIN UPDATE thread_generation SET revision=revision+1; END;
CREATE TRIGGER thread_topology_delete AFTER DELETE ON provider_documents
WHEN OLD.key='probe' OR OLD.key='thread:folders' OR substr(OLD.key,1,13)='thread:epoch:'
BEGIN UPDATE thread_generation SET revision=revision+1; END;
CREATE TABLE thread_edges (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    edge TEXT NOT NULL,
    PRIMARY KEY(edge, message_id)
);
CREATE INDEX thread_edges_message ON thread_edges(message_id);
INSERT OR IGNORE INTO thread_edges(message_id,edge)
SELECT m.id, 'rfc:' || j.value FROM messages m
JOIN provider_documents d ON d.key='thread-headers:' || m.id,
json_each(d.document,'$.ids') j WHERE j.key<256;
INSERT OR IGNORE INTO thread_edges(message_id,edge)
SELECT m.id, 'gmail:' || json_extract(d.document,'$.gmail') FROM messages m
JOIN provider_documents d ON d.key='thread-headers:' || m.id
WHERE json_extract(d.document,'$.gmail') IS NOT NULL;
CREATE TRIGGER thread_headers_insert AFTER INSERT ON provider_documents
WHEN substr(NEW.key,1,15)='thread-headers:' BEGIN
    UPDATE thread_generation SET revision=revision+1;
    INSERT OR IGNORE INTO thread_edges(message_id,edge)
    SELECT substr(NEW.key,16), 'rfc:' || value FROM json_each(NEW.document,'$.ids') WHERE key<256;
    INSERT OR IGNORE INTO thread_edges(message_id,edge)
    SELECT substr(NEW.key,16), 'gmail:' || json_extract(NEW.document,'$.gmail')
    WHERE json_extract(NEW.document,'$.gmail') IS NOT NULL;
END;
CREATE TRIGGER thread_headers_update AFTER UPDATE OF document ON provider_documents
WHEN substr(NEW.key,1,15)='thread-headers:' AND OLD.document<>NEW.document BEGIN
    UPDATE thread_generation SET revision=revision+1;
    DELETE FROM thread_edges WHERE message_id=substr(NEW.key,16);
    INSERT OR IGNORE INTO thread_edges(message_id,edge)
    SELECT substr(NEW.key,16), 'rfc:' || value FROM json_each(NEW.document,'$.ids') WHERE key<256;
    INSERT OR IGNORE INTO thread_edges(message_id,edge)
    SELECT substr(NEW.key,16), 'gmail:' || json_extract(NEW.document,'$.gmail')
    WHERE json_extract(NEW.document,'$.gmail') IS NOT NULL;
END;
CREATE TRIGGER thread_headers_delete AFTER DELETE ON provider_documents
WHEN substr(OLD.key,1,15)='thread-headers:' BEGIN
    UPDATE thread_generation SET revision=revision+1;
    DELETE FROM thread_edges WHERE message_id=substr(OLD.key,16);
END;
