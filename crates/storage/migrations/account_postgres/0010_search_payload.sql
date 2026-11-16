ALTER TABLE search_documents ADD COLUMN body_payload BIGINT NOT NULL DEFAULT 0 CHECK(body_payload IN (0,1));
UPDATE search_documents SET body_payload=1;
CREATE OR REPLACE FUNCTION search_document_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN PERFORM publish_search_change(OLD.account_id,OLD.message_id); RETURN OLD; END IF;
 IF TG_OP='UPDATE' AND OLD.body_payload=1 AND NEW.body_payload=0 AND OLD.source=NEW.source AND OLD.body_state=NEW.body_state AND OLD.sender=NEW.sender AND OLD.recipients=NEW.recipients AND OLD.subject=NEW.subject AND OLD.received_at=NEW.received_at THEN RETURN NEW; END IF;
 PERFORM publish_search_change(NEW.account_id,NEW.message_id); RETURN NEW;
END $$;
