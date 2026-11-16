-- Literal metadata substrings use a small trigram candidate prefix, followed
-- by an exact normalized substring check. Body text has separate lexical jobs.
CREATE VIRTUAL TABLE metadata_terms USING fts5(sender,recipients,subject,content=documents,content_rowid=rowid,tokenize='trigram case_sensitive 1');
INSERT INTO metadata_terms(metadata_terms,rank) VALUES('secure-delete',1);
INSERT INTO metadata_terms(rowid,sender,recipients,subject) SELECT rowid,sender,recipients,subject FROM documents;
CREATE TRIGGER metadata_terms_insert AFTER INSERT ON documents BEGIN
 INSERT INTO metadata_terms(rowid,sender,recipients,subject) VALUES(new.rowid,new.sender,new.recipients,new.subject);
END;
CREATE TRIGGER metadata_terms_delete AFTER DELETE ON documents BEGIN
 INSERT INTO metadata_terms(metadata_terms,rowid,sender,recipients,subject) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject);
END;
CREATE TRIGGER metadata_terms_update AFTER UPDATE OF sender,recipients,subject ON documents
WHEN old.sender<>new.sender OR old.recipients<>new.recipients OR old.subject<>new.subject BEGIN
 INSERT INTO metadata_terms(metadata_terms,rowid,sender,recipients,subject) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject);
 INSERT INTO metadata_terms(rowid,sender,recipients,subject) VALUES(new.rowid,new.sender,new.recipients,new.subject);
END;
