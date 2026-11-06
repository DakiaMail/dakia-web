-- Search text is durable account-private data, independent of the MIME cache.
CREATE TABLE search_documents (
    rowid INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL UNIQUE REFERENCES messages(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    sender TEXT NOT NULL,
    recipients TEXT NOT NULL,
    subject TEXT NOT NULL,
    received_at TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    body_state TEXT NOT NULL DEFAULT 'pending',
    retry_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX search_backlog ON search_documents(body_state,retry_at,message_id);
CREATE INDEX search_order ON search_documents(received_at DESC,message_id DESC);
CREATE TABLE search_totals(id INTEGER PRIMARY KEY CHECK(id=1),body_bytes INTEGER NOT NULL DEFAULT 0);
INSERT INTO search_totals(id) VALUES(1);
CREATE VIRTUAL TABLE search_fts USING fts5(sender,recipients,subject,body,content=search_documents,content_rowid=rowid,tokenize='trigram case_sensitive 1');
INSERT INTO search_fts(search_fts,rank) VALUES('secure-delete',1);
CREATE TRIGGER search_insert AFTER INSERT ON search_documents BEGIN
    UPDATE search_totals SET body_bytes=body_bytes+length(CAST(new.body AS BLOB)) WHERE id=1;
    INSERT INTO search_fts(rowid,sender,recipients,subject,body) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body);
END;
CREATE TRIGGER search_delete AFTER DELETE ON search_documents BEGIN
    UPDATE search_totals SET body_bytes=body_bytes-length(CAST(old.body AS BLOB)) WHERE id=1;
    INSERT INTO search_fts(search_fts,rowid,sender,recipients,subject,body) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body);
END;
CREATE TRIGGER search_update AFTER UPDATE OF sender,recipients,subject,body ON search_documents BEGIN
    UPDATE search_totals SET body_bytes=body_bytes+length(CAST(new.body AS BLOB))-length(CAST(old.body AS BLOB)) WHERE id=1;
    INSERT INTO search_fts(search_fts,rowid,sender,recipients,subject,body) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body);
    INSERT INTO search_fts(rowid,sender,recipients,subject,body) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body);
END;
-- All metadata paths, including reconciliation, use this idempotent queue.
CREATE TABLE search_dirty(message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE);
INSERT INTO search_dirty SELECT id FROM messages;
CREATE TRIGGER search_message_insert AFTER INSERT ON messages BEGIN
    INSERT INTO search_dirty SELECT new.id WHERE NOT EXISTS(SELECT 1 FROM search_dirty WHERE message_id=new.id);
END;
CREATE TRIGGER search_message_update AFTER UPDATE OF document ON messages BEGIN
    INSERT INTO search_dirty SELECT new.id WHERE NOT EXISTS(SELECT 1 FROM search_dirty WHERE message_id=new.id);
    DELETE FROM search_documents WHERE message_id=new.id AND json_array_length(new.document,'$.folder_ids')=0;
END;
-- UID epoch changes revoke old generic IMAP content immediately, not at the
-- next history scan. Gmail stable IDs are not folder/epoch identities.
CREATE TRIGGER search_epoch_update AFTER UPDATE OF document ON provider_documents WHEN new.key GLOB 'thread:epoch:*' BEGIN
    DELETE FROM search_documents WHERE message_id IN (
        SELECT id FROM messages WHERE json_valid(json_extract(document,'$.provider_id'))
        AND json_extract(json_extract(document,'$.provider_id'),'$.folder')=substr(new.key,14)
        AND json_extract(json_extract(document,'$.provider_id'),'$.uidvalidity')<>CAST(new.document AS INTEGER)
    );
END;
