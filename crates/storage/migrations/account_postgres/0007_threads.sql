ALTER TABLE account_locks ADD COLUMN thread_revision BIGINT NOT NULL DEFAULT 0;
CREATE TABLE thread_verifications (
 account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,
 component TEXT NOT NULL,revision BIGINT NOT NULL,checked_at BIGINT NOT NULL,
 PRIMARY KEY(account_id,component)
);
CREATE TABLE thread_edges (
 account_id TEXT NOT NULL,message_id TEXT NOT NULL,edge TEXT NOT NULL,
 PRIMARY KEY(account_id,edge,message_id),
 FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE
);
CREATE INDEX thread_edges_message ON thread_edges(account_id,message_id);
-- Provider state also holds opaque non-JSON receipts. Materialize the header
-- subset before decoding, so the planner cannot evaluate JSON casts on those.
WITH headers AS MATERIALIZED (
 SELECT account_id,substring(key FROM 16) message_id,document::jsonb payload
 FROM provider_documents WHERE left(key,15)='thread-headers:'
)
INSERT INTO thread_edges(account_id,message_id,edge)
 SELECT m.account_id,m.id,'rfc:'||value FROM messages m JOIN headers h ON h.account_id=m.account_id AND h.message_id=m.id
 CROSS JOIN LATERAL jsonb_array_elements_text(h.payload->'ids') WITH ORDINALITY entries(value,entry_number) WHERE entry_number<=256 ON CONFLICT DO NOTHING;
WITH headers AS MATERIALIZED (
 SELECT account_id,substring(key FROM 16) message_id,document::jsonb payload
 FROM provider_documents WHERE left(key,15)='thread-headers:'
)
INSERT INTO thread_edges(account_id,message_id,edge)
 SELECT m.account_id,m.id,'gmail:'||(h.payload->>'gmail') FROM messages m JOIN headers h ON h.account_id=m.account_id AND h.message_id=m.id
 WHERE h.payload->>'gmail' IS NOT NULL ON CONFLICT DO NOTHING;
CREATE FUNCTION thread_message_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN
  UPDATE account_locks SET thread_revision=thread_revision+1 WHERE account_id=OLD.account_id; RETURN OLD;
 END IF;
 IF TG_OP='UPDATE' AND OLD.source IS NOT DISTINCT FROM NEW.source AND OLD.folder_ids IS NOT DISTINCT FROM NEW.folder_ids AND OLD.thread_id IS NOT DISTINCT FROM NEW.thread_id THEN RETURN NEW; END IF;
 UPDATE account_locks SET thread_revision=thread_revision+1 WHERE account_id=NEW.account_id; RETURN NEW;
END $$;
CREATE TRIGGER thread_message_insert AFTER INSERT ON messages FOR EACH ROW EXECUTE FUNCTION thread_message_changed();
CREATE TRIGGER thread_message_update AFTER UPDATE OF document ON messages FOR EACH ROW EXECUTE FUNCTION thread_message_changed();
CREATE TRIGGER thread_message_delete AFTER DELETE ON messages FOR EACH ROW EXECUTE FUNCTION thread_message_changed();
CREATE FUNCTION thread_evidence_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
DECLARE owner_account TEXT; name TEXT; topology BOOLEAN; header BOOLEAN; identity TEXT;
BEGIN
 IF TG_OP='DELETE' THEN owner_account=OLD.account_id;name=OLD.key;
 ELSE owner_account=NEW.account_id;name=NEW.key; END IF;
 header=left(name,15)='thread-headers:';
 topology=name IN ('probe','thread:folders') OR left(name,13)='thread:epoch:';
 IF NOT header AND NOT topology THEN RETURN NULL; END IF;
 IF TG_OP='UPDATE' THEN
  IF header AND OLD.document=NEW.document THEN RETURN NEW; END IF;
  IF NOT header AND NOT (
   (name='thread:folders' AND OLD.document::jsonb->'names' IS DISTINCT FROM NEW.document::jsonb->'names') OR
   (left(name,13)='thread:epoch:' AND OLD.document<>NEW.document) OR
   (name='probe' AND OLD.document::jsonb->'full_search_coverage' IS DISTINCT FROM NEW.document::jsonb->'full_search_coverage')
  ) THEN RETURN NEW; END IF;
 END IF;
 UPDATE account_locks SET thread_revision=thread_revision+1 WHERE account_id=owner_account;
 IF header THEN
  identity=substring(name FROM 16);
  DELETE FROM thread_edges WHERE account_id=owner_account AND message_id=identity;
  IF TG_OP<>'DELETE' THEN
   INSERT INTO thread_edges(account_id,message_id,edge)
    SELECT owner_account,identity,'rfc:'||value FROM jsonb_array_elements_text(NEW.document::jsonb->'ids') WITH ORDINALITY entries(value,entry_number) WHERE entry_number<=256 ON CONFLICT DO NOTHING;
   INSERT INTO thread_edges(account_id,message_id,edge)
    SELECT owner_account,identity,'gmail:'||(NEW.document::jsonb->>'gmail') WHERE NEW.document::jsonb->>'gmail' IS NOT NULL ON CONFLICT DO NOTHING;
  END IF;
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER thread_evidence_insert AFTER INSERT ON provider_documents FOR EACH ROW EXECUTE FUNCTION thread_evidence_changed();
CREATE TRIGGER thread_evidence_update AFTER UPDATE OF document ON provider_documents FOR EACH ROW EXECUTE FUNCTION thread_evidence_changed();
CREATE TRIGGER thread_evidence_delete AFTER DELETE ON provider_documents FOR EACH ROW EXECUTE FUNCTION thread_evidence_changed();
