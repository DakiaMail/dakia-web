CREATE TABLE documents (
 rowid INTEGER PRIMARY KEY, account_id TEXT NOT NULL, message_id TEXT NOT NULL,
 version INTEGER NOT NULL, source TEXT NOT NULL, metadata TEXT NOT NULL,
 sender TEXT NOT NULL, recipients TEXT NOT NULL, subject TEXT NOT NULL,
 body TEXT NOT NULL, participants TEXT NOT NULL, attachment_text TEXT NOT NULL,
 received_at TEXT NOT NULL, state TEXT NOT NULL,
 UNIQUE(account_id,message_id)
);
CREATE INDEX documents_account_order ON documents(account_id,received_at DESC,message_id DESC);
CREATE VIRTUAL TABLE terms USING fts5(sender,recipients,subject,body,participants,attachment_text,content=documents,content_rowid=rowid,tokenize='unicode61 remove_diacritics 2');
INSERT INTO terms(terms,rank) VALUES('secure-delete',1);
CREATE TRIGGER terms_insert AFTER INSERT ON documents BEGIN
 INSERT INTO terms(rowid,sender,recipients,subject,body,participants,attachment_text) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body,new.participants,new.attachment_text);
END;
CREATE TRIGGER terms_delete AFTER DELETE ON documents BEGIN
 INSERT INTO terms(terms,rowid,sender,recipients,subject,body,participants,attachment_text) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body,old.participants,old.attachment_text);
END;
CREATE TRIGGER terms_update AFTER UPDATE OF sender,recipients,subject,body,participants,attachment_text ON documents BEGIN
 INSERT INTO terms(terms,rowid,sender,recipients,subject,body,participants,attachment_text) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body,old.participants,old.attachment_text);
 INSERT INTO terms(rowid,sender,recipients,subject,body,participants,attachment_text) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body,new.participants,new.attachment_text);
END;
CREATE TABLE chunks (
 id INTEGER PRIMARY KEY,account_id TEXT NOT NULL,message_id TEXT NOT NULL,
 source TEXT NOT NULL,model_revision TEXT NOT NULL,ordinal INTEGER NOT NULL,text TEXT NOT NULL,
 embedding BLOB NOT NULL,UNIQUE(account_id,message_id,model_revision,ordinal)
);
CREATE INDEX chunks_message ON chunks(account_id,message_id);
CREATE TABLE index_identity (id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL);
INSERT INTO index_identity VALUES(1,lower(hex(randomblob(16))));
