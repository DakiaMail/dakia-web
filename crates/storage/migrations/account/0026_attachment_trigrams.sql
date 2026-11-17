CREATE TABLE IF NOT EXISTS search_runtime_mode(id INTEGER PRIMARY KEY CHECK(id=1),external INTEGER NOT NULL DEFAULT 0,legacy_ready INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO search_runtime_mode(id) VALUES(1);
DROP TRIGGER search_insert;
DROP TRIGGER search_delete;
DROP TRIGGER search_update;
DROP TABLE search_fts;
CREATE VIRTUAL TABLE search_fts USING fts5(sender,recipients,subject,body,participants,mime_recipients,attachment_text,content=search_documents,content_rowid=rowid,tokenize='trigram case_sensitive 1');
INSERT INTO search_fts(search_fts,rank) VALUES('secure-delete',1);
CREATE TRIGGER search_insert AFTER INSERT ON search_documents BEGIN
 UPDATE search_totals SET body_bytes=body_bytes+length(CAST(new.body AS BLOB))+length(CAST(new.attachment_text AS BLOB)) WHERE id=1;
 INSERT INTO search_fts(rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body,new.participants,new.mime_recipients,new.attachment_text);
END;
CREATE TRIGGER search_delete AFTER DELETE ON search_documents BEGIN
 UPDATE search_totals SET body_bytes=body_bytes-length(CAST(old.body AS BLOB))-length(CAST(old.attachment_text AS BLOB)) WHERE id=1;
 INSERT INTO search_fts(search_fts,rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body,old.participants,old.mime_recipients,old.attachment_text);
END;
CREATE TRIGGER search_update AFTER UPDATE OF sender,recipients,subject,body,participants,mime_recipients,attachment_text ON search_documents BEGIN
 UPDATE search_totals SET body_bytes=body_bytes+length(CAST(new.body AS BLOB))+length(CAST(new.attachment_text AS BLOB))-length(CAST(old.body AS BLOB))-length(CAST(old.attachment_text AS BLOB)) WHERE id=1;
 INSERT INTO search_fts(search_fts,rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body,old.participants,old.mime_recipients,old.attachment_text);
 INSERT INTO search_fts(rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body,new.participants,new.mime_recipients,new.attachment_text);
END;
INSERT INTO search_fts(search_fts) SELECT 'rebuild' WHERE (SELECT external FROM search_runtime_mode WHERE id=1)=0;
UPDATE search_runtime_mode SET legacy_ready=CASE WHEN external=0 THEN 1 ELSE 0 END WHERE id=1;
