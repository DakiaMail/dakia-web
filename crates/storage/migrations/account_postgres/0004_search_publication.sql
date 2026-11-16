CREATE FUNCTION search_document_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN
  PERFORM publish_search_change(OLD.account_id,OLD.message_id); RETURN OLD;
 END IF;
 PERFORM publish_search_change(NEW.account_id,NEW.message_id); RETURN NEW;
END $$;
CREATE TRIGGER search_document_insert AFTER INSERT ON search_documents FOR EACH ROW EXECUTE FUNCTION search_document_changed();
CREATE TRIGGER search_document_delete AFTER DELETE ON search_documents FOR EACH ROW EXECUTE FUNCTION search_document_changed();
CREATE TRIGGER search_document_update AFTER UPDATE OF source,sender,recipients,subject,received_at,body,body_state,participants,mime_recipients,attachment_text,attachment_details ON search_documents FOR EACH ROW EXECUTE FUNCTION search_document_changed();
CREATE FUNCTION search_delivery_pending() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 UPDATE search_documents SET delivered=0 WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 RETURN NEW;
END $$;
CREATE TRIGGER search_receipt_pending AFTER INSERT OR UPDATE ON search_index_outbox FOR EACH ROW EXECUTE FUNCTION search_delivery_pending();
CREATE FUNCTION search_epoch_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
DECLARE owner_account TEXT; folder_name TEXT; identity TEXT;
BEGIN
 IF TG_OP='DELETE' THEN owner_account=OLD.account_id;folder_name=substring(OLD.key FROM 14);
 ELSE
  IF TG_OP='UPDATE' AND OLD.document=NEW.document THEN RETURN NEW; END IF;
  owner_account=NEW.account_id;folder_name=substring(NEW.key FROM 14);
 END IF;
 INSERT INTO search_dirty(account_id,message_id) SELECT account_id,id FROM messages WHERE account_id=owner_account AND provider_folder=folder_name ON CONFLICT DO NOTHING;
 FOR identity IN SELECT id FROM messages WHERE account_id=owner_account AND provider_folder=folder_name LOOP
  PERFORM publish_search_change(owner_account,identity);
 END LOOP;
 IF TG_OP<>'DELETE' THEN
  DELETE FROM search_documents d USING messages m WHERE d.account_id=owner_account AND m.account_id=d.account_id AND m.id=d.message_id AND m.provider_folder=folder_name AND m.provider_epoch<>NEW.document::BIGINT;
  RETURN NEW;
 END IF;
 RETURN OLD;
END $$;
CREATE TRIGGER search_epoch_insert AFTER INSERT ON provider_documents FOR EACH ROW WHEN (left(NEW.key,13)='thread:epoch:') EXECUTE FUNCTION search_epoch_changed();
CREATE TRIGGER search_epoch_update AFTER UPDATE OF document ON provider_documents FOR EACH ROW WHEN (left(NEW.key,13)='thread:epoch:') EXECUTE FUNCTION search_epoch_changed();
CREATE TRIGGER search_epoch_delete AFTER DELETE ON provider_documents FOR EACH ROW WHEN (left(OLD.key,13)='thread:epoch:') EXECUTE FUNCTION search_epoch_changed();
CREATE FUNCTION search_empty_membership() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF cardinality(NEW.folder_ids)=0 THEN DELETE FROM search_documents WHERE account_id=NEW.account_id AND message_id=NEW.id; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER search_empty_membership AFTER UPDATE OF document ON messages FOR EACH ROW EXECUTE FUNCTION search_empty_membership();
