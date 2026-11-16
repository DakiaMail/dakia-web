-- Received text remains a source-guarded acquisition stage. Its bookkeeping
-- must not depend on a healthy text index. The server removes the legacy FTS
-- table when using an independent SQLite/PostgreSQL search store.
DROP TRIGGER search_insert;
DROP TRIGGER search_delete;
DROP TRIGGER search_update;
CREATE TRIGGER search_bytes_insert AFTER INSERT ON search_documents BEGIN
 UPDATE search_totals SET body_bytes=body_bytes+length(CAST(new.body AS BLOB))+length(CAST(new.attachment_text AS BLOB)) WHERE id=1;
END;
CREATE TRIGGER search_bytes_delete AFTER DELETE ON search_documents BEGIN
 UPDATE search_totals SET body_bytes=body_bytes-length(CAST(old.body AS BLOB))-length(CAST(old.attachment_text AS BLOB)) WHERE id=1;
END;
CREATE TRIGGER search_bytes_update AFTER UPDATE OF body,attachment_text ON search_documents BEGIN
 UPDATE search_totals SET body_bytes=body_bytes+length(CAST(new.body AS BLOB))+length(CAST(new.attachment_text AS BLOB))-length(CAST(old.body AS BLOB))-length(CAST(old.attachment_text AS BLOB)) WHERE id=1;
END;
CREATE TRIGGER search_fts_insert AFTER INSERT ON search_documents BEGIN
 INSERT INTO search_fts(rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body,new.participants,new.mime_recipients,new.attachment_text);
END;
CREATE TRIGGER search_fts_delete AFTER DELETE ON search_documents BEGIN
 INSERT INTO search_fts(search_fts,rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body,old.participants,old.mime_recipients,old.attachment_text);
END;
CREATE TRIGGER search_fts_update AFTER UPDATE OF sender,recipients,subject,body,participants,mime_recipients,attachment_text ON search_documents BEGIN
 INSERT INTO search_fts(search_fts,rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body,old.participants,old.mime_recipients,old.attachment_text);
 INSERT INTO search_fts(rowid,sender,recipients,subject,body,participants,mime_recipients,attachment_text) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body,new.participants,new.mime_recipients,new.attachment_text);
END;
