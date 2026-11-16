ALTER TABLE documents ADD COLUMN unread BOOLEAN GENERATED ALWAYS AS (COALESCE((metadata::jsonb->>'unread')::BOOLEAN,FALSE)) STORED;
ALTER TABLE documents ADD COLUMN starred BOOLEAN GENERATED ALWAYS AS (COALESCE((metadata::jsonb->>'starred')::BOOLEAN,FALSE)) STORED;
ALTER TABLE documents ADD COLUMN category TEXT GENERATED ALWAYS AS (metadata::jsonb->>'category') STORED;
ALTER TABLE documents ADD COLUMN snoozed_until TEXT;
CREATE INDEX documents_unread ON documents(account_id,unread,received_at DESC,message_id COLLATE "C" DESC) WHERE state<>'deleted';
CREATE INDEX documents_starred ON documents(account_id,starred,received_at DESC,message_id COLLATE "C" DESC) WHERE state<>'deleted';
CREATE INDEX documents_category ON documents(account_id,category,received_at DESC,message_id COLLATE "C" DESC) WHERE state<>'deleted';
CREATE INDEX documents_snoozed ON documents(account_id,snoozed_until,message_id) WHERE state<>'deleted';
CREATE TABLE document_folders(account_id TEXT NOT NULL,folder TEXT NOT NULL,message_id TEXT NOT NULL,PRIMARY KEY(account_id,folder,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE);
CREATE INDEX document_folders_message ON document_folders(account_id,message_id);
INSERT INTO document_folders SELECT DISTINCT d.account_id,f.value,d.message_id FROM documents d CROSS JOIN LATERAL jsonb_array_elements_text(d.metadata::jsonb->'folder_ids') f(value);
CREATE FUNCTION document_folders_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='UPDATE' AND OLD.metadata::jsonb->'folder_ids' IS NOT DISTINCT FROM NEW.metadata::jsonb->'folder_ids' THEN RETURN NEW;END IF;
 DELETE FROM document_folders WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO document_folders SELECT DISTINCT NEW.account_id,value,NEW.message_id FROM jsonb_array_elements_text(NEW.metadata::jsonb->'folder_ids') value;
 RETURN NEW;
END $$;
CREATE TRIGGER document_folders_changed AFTER INSERT OR UPDATE OF metadata ON documents FOR EACH ROW EXECUTE FUNCTION document_folders_changed();
