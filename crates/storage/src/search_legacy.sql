CREATE VIRTUAL TABLE search_fts USING fts5(sender,recipients,subject,body,participants,mime_recipients,attachment_text,content=search_documents,content_rowid=rowid,tokenize='trigram case_sensitive 1');
INSERT INTO search_fts(search_fts,rank) VALUES('secure-delete',1);
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
INSERT INTO search_fts(search_fts) VALUES('rebuild');
