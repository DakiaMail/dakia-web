ALTER TABLE documents ADD COLUMN metadata_recipients TEXT NOT NULL DEFAULT '';
ALTER TABLE documents ADD COLUMN mime_recipients TEXT NOT NULL DEFAULT '';
ALTER TABLE documents ADD COLUMN attachment_details TEXT NOT NULL DEFAULT '[]';
ALTER TABLE documents ADD COLUMN body_payload INTEGER NOT NULL DEFAULT 1;
-- Legacy merged recipients remain searchable until fresh extraction separates
-- the MIME fields. A metadata patch must not discard existing Cc/Bcc evidence.
UPDATE documents SET mime_recipients=recipients;
DROP TRIGGER terms_update;
CREATE TRIGGER terms_update AFTER UPDATE OF sender,recipients,subject,body,participants,attachment_text ON documents
WHEN old.sender<>new.sender OR old.recipients<>new.recipients OR old.subject<>new.subject OR old.body<>new.body OR old.participants<>new.participants OR old.attachment_text<>new.attachment_text BEGIN
 INSERT INTO terms(terms,rowid,sender,recipients,subject,body,participants,attachment_text) VALUES('delete',old.rowid,old.sender,old.recipients,old.subject,old.body,old.participants,old.attachment_text);
 INSERT INTO terms(rowid,sender,recipients,subject,body,participants,attachment_text) VALUES(new.rowid,new.sender,new.recipients,new.subject,new.body,new.participants,new.attachment_text);
END;
