ALTER TABLE documents ADD COLUMN unread INTEGER GENERATED ALWAYS AS (COALESCE(json_extract(metadata,'$.unread'),0)) VIRTUAL;
ALTER TABLE documents ADD COLUMN starred INTEGER GENERATED ALWAYS AS (COALESCE(json_extract(metadata,'$.starred'),0)) VIRTUAL;
ALTER TABLE documents ADD COLUMN category TEXT GENERATED ALWAYS AS (json_extract(metadata,'$.category')) VIRTUAL;
ALTER TABLE documents ADD COLUMN snoozed_until TEXT;
ALTER TABLE documents ADD COLUMN filter_revision INTEGER NOT NULL DEFAULT 0;
CREATE INDEX documents_unread ON documents(account_id,unread,received_at DESC,message_id DESC) WHERE state<>'deleted';
CREATE INDEX documents_starred ON documents(account_id,starred,received_at DESC,message_id DESC) WHERE state<>'deleted';
CREATE INDEX documents_category ON documents(account_id,category,received_at DESC,message_id DESC) WHERE state<>'deleted';
CREATE INDEX documents_snoozed ON documents(account_id,snoozed_until,message_id) WHERE state<>'deleted';
CREATE TABLE document_folders(account_id TEXT NOT NULL,folder TEXT NOT NULL,message_id TEXT NOT NULL,PRIMARY KEY(account_id,folder,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE);
CREATE INDEX document_folders_message ON document_folders(account_id,message_id);
INSERT INTO document_folders SELECT d.account_id,f.value,d.message_id FROM documents d,json_each(d.metadata,'$.folder_ids') f GROUP BY d.account_id,f.value,d.message_id;
CREATE TRIGGER document_folders_insert AFTER INSERT ON documents BEGIN INSERT OR IGNORE INTO document_folders SELECT NEW.account_id,value,NEW.message_id FROM json_each(NEW.metadata,'$.folder_ids');END;
CREATE TRIGGER document_folders_update AFTER UPDATE OF metadata ON documents WHEN json_extract(OLD.metadata,'$.folder_ids') IS NOT json_extract(NEW.metadata,'$.folder_ids') BEGIN
 DELETE FROM document_folders WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT OR IGNORE INTO document_folders SELECT NEW.account_id,value,NEW.message_id FROM json_each(NEW.metadata,'$.folder_ids');
END;
ALTER TABLE embedding_changes ADD COLUMN metadata_only INTEGER NOT NULL DEFAULT 0;
-- Reuse the durable vector frontier for changes to candidate eligibility.
CREATE TRIGGER vector_metadata_changed AFTER UPDATE OF metadata,sender,recipients,subject,snoozed_until,content_version,state ON documents WHEN OLD.metadata<>NEW.metadata OR OLD.sender<>NEW.sender OR OLD.recipients<>NEW.recipients OR OLD.subject<>NEW.subject OR OLD.snoozed_until IS NOT NEW.snoozed_until OR OLD.content_version<>NEW.content_version OR OLD.state<>NEW.state BEGIN
 UPDATE documents SET filter_revision=filter_revision+1 WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO embedding_changes(chunk_id,account_id,model_revision,metadata_only) SELECT id,account_id,model_revision,1 FROM chunks WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
END;
