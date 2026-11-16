-- Source inventory is independent of the selected search service. Eligibility
-- is materialized per message so membership/epoch changes update exact counters
-- without a mailbox recount on each status poll or search.
CREATE TABLE search_totals (
 account_id TEXT PRIMARY KEY REFERENCES account_locks(account_id) ON DELETE CASCADE,
 headers BIGINT NOT NULL DEFAULT 0,
 bodies BIGINT NOT NULL DEFAULT 0,
 pending BIGINT NOT NULL DEFAULT 0,
 gaps BIGINT NOT NULL DEFAULT 0,
 body_bytes BIGINT NOT NULL DEFAULT 0,
 retrying BIGINT NOT NULL DEFAULT 0,
 processed BIGINT NOT NULL DEFAULT 0,
 dirty BIGINT NOT NULL DEFAULT 0,
 delivery_pending BIGINT NOT NULL DEFAULT 0,
 eligible BIGINT NOT NULL DEFAULT 0
);
INSERT INTO search_totals(account_id) SELECT account_id FROM account_locks;
CREATE FUNCTION search_inventory_account() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN INSERT INTO search_totals(account_id) VALUES(NEW.account_id);RETURN NEW;END $$;
CREATE TRIGGER search_inventory_account AFTER INSERT ON account_locks FOR EACH ROW EXECUTE FUNCTION search_inventory_account();
CREATE TABLE search_inventory (
 account_id TEXT NOT NULL,message_id TEXT NOT NULL,eligible BOOLEAN NOT NULL,
 PRIMARY KEY(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE
);
INSERT INTO search_inventory SELECT m.account_id,m.id,EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=m.account_id AND e.id=m.id) FROM messages m;
CREATE FUNCTION search_inventory_count() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN UPDATE search_totals SET eligible=eligible-OLD.eligible::INTEGER WHERE account_id=OLD.account_id; RETURN OLD; END IF;
 IF TG_OP='INSERT' THEN UPDATE search_totals SET eligible=eligible+NEW.eligible::INTEGER WHERE account_id=NEW.account_id;
 ELSE UPDATE search_totals SET eligible=eligible+NEW.eligible::INTEGER-OLD.eligible::INTEGER WHERE account_id=NEW.account_id;END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER search_inventory_count AFTER INSERT OR UPDATE OR DELETE ON search_inventory FOR EACH ROW EXECUTE FUNCTION search_inventory_count();
CREATE FUNCTION search_inventory_message() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 INSERT INTO search_inventory(account_id,message_id,eligible) VALUES(NEW.account_id,NEW.id,EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=NEW.account_id AND e.id=NEW.id)) ON CONFLICT(account_id,message_id) DO UPDATE SET eligible=excluded.eligible WHERE search_inventory.eligible<>excluded.eligible;
 -- Confirmed exclusion removes retained source text immediately, independently
 -- of header scheduling or the search service's availability.
 IF NOT EXISTS(SELECT 1 FROM search_inventory WHERE account_id=NEW.account_id AND message_id=NEW.id AND eligible) THEN DELETE FROM search_documents WHERE account_id=NEW.account_id AND message_id=NEW.id; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER search_inventory_message_insert AFTER INSERT ON messages FOR EACH ROW EXECUTE FUNCTION search_inventory_message();
CREATE TRIGGER search_inventory_message_update AFTER UPDATE OF document ON messages FOR EACH ROW EXECUTE FUNCTION search_inventory_message();
CREATE FUNCTION search_inventory_epoch() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
DECLARE owner_account TEXT;folder_name TEXT;
BEGIN
 IF TG_OP='DELETE' THEN owner_account=OLD.account_id;folder_name=substring(OLD.key FROM 14);ELSE owner_account=NEW.account_id;folder_name=substring(NEW.key FROM 14);END IF;
 UPDATE search_inventory i SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=i.account_id AND e.id=i.message_id) FROM messages m WHERE i.account_id=owner_account AND m.account_id=i.account_id AND m.id=i.message_id AND m.provider_folder=folder_name;
 IF TG_OP='DELETE' THEN RETURN OLD;END IF; RETURN NEW;
END $$;
CREATE TRIGGER search_inventory_epoch_insert AFTER INSERT ON provider_documents FOR EACH ROW WHEN(left(NEW.key,13)='thread:epoch:') EXECUTE FUNCTION search_inventory_epoch();
CREATE TRIGGER search_inventory_epoch_update AFTER UPDATE OF document ON provider_documents FOR EACH ROW WHEN(left(NEW.key,13)='thread:epoch:') EXECUTE FUNCTION search_inventory_epoch();
CREATE TRIGGER search_inventory_epoch_delete AFTER DELETE ON provider_documents FOR EACH ROW WHEN(left(OLD.key,13)='thread:epoch:') EXECUTE FUNCTION search_inventory_epoch();
CREATE FUNCTION search_source_counts() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP='INSERT' THEN
 UPDATE search_totals SET headers=headers+1,bodies=bodies+(NEW.body_state='ready')::INTEGER,pending=pending+(NEW.body_state='pending')::INTEGER,gaps=gaps+(NEW.body_state NOT IN ('pending','ready'))::INTEGER,body_bytes=body_bytes+(octet_length(NEW.body)+octet_length(NEW.attachment_text)),retrying=retrying+(NEW.body_state='fetch_error')::INTEGER,processed=processed+(NEW.delivered=1 AND NEW.body_state NOT IN ('pending','fetch_error','quota'))::INTEGER WHERE account_id=NEW.account_id;RETURN NEW;
 END IF;
 IF TG_OP='DELETE' THEN
 UPDATE search_totals SET headers=headers-1,bodies=bodies-(OLD.body_state='ready')::INTEGER,pending=pending-(OLD.body_state='pending')::INTEGER,gaps=gaps-(OLD.body_state NOT IN ('pending','ready'))::INTEGER,body_bytes=body_bytes-(octet_length(OLD.body)+octet_length(OLD.attachment_text)),retrying=retrying-(OLD.body_state='fetch_error')::INTEGER,processed=processed-(OLD.delivered=1 AND OLD.body_state NOT IN ('pending','fetch_error','quota'))::INTEGER WHERE account_id=OLD.account_id;RETURN OLD;
 END IF;
 UPDATE search_totals SET headers=headers+1-1,bodies=bodies+(NEW.body_state='ready')::INTEGER-(OLD.body_state='ready')::INTEGER,pending=pending+(NEW.body_state='pending')::INTEGER-(OLD.body_state='pending')::INTEGER,gaps=gaps+(NEW.body_state NOT IN ('pending','ready'))::INTEGER-(OLD.body_state NOT IN ('pending','ready'))::INTEGER,body_bytes=body_bytes+(octet_length(NEW.body)+octet_length(NEW.attachment_text))-(octet_length(OLD.body)+octet_length(OLD.attachment_text)),retrying=retrying+(NEW.body_state='fetch_error')::INTEGER-(OLD.body_state='fetch_error')::INTEGER,processed=processed+(NEW.delivered=1 AND NEW.body_state NOT IN ('pending','fetch_error','quota'))::INTEGER-(OLD.delivered=1 AND OLD.body_state NOT IN ('pending','fetch_error','quota'))::INTEGER WHERE account_id=NEW.account_id;RETURN NEW;
END $$;
CREATE TRIGGER search_source_counts AFTER INSERT OR UPDATE OR DELETE ON search_documents FOR EACH ROW EXECUTE FUNCTION search_source_counts();
CREATE FUNCTION search_queue_counts() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
DECLARE owner_account TEXT;delta INTEGER;
BEGIN
 IF TG_OP='INSERT' THEN owner_account=NEW.account_id;delta=1;ELSE owner_account=OLD.account_id;delta=-1;END IF;
 IF TG_TABLE_NAME='search_dirty' THEN UPDATE search_totals SET dirty=dirty+delta WHERE account_id=owner_account;
 ELSE UPDATE search_totals SET delivery_pending=delivery_pending+delta WHERE account_id=owner_account;END IF;
 IF TG_OP='DELETE' THEN RETURN OLD;END IF;RETURN NEW;
END $$;
CREATE TRIGGER search_dirty_counts AFTER INSERT OR DELETE ON search_dirty FOR EACH ROW EXECUTE FUNCTION search_queue_counts();
CREATE TRIGGER search_outbox_counts AFTER INSERT OR DELETE ON search_index_outbox FOR EACH ROW EXECUTE FUNCTION search_queue_counts();
UPDATE search_totals t SET headers=(SELECT COALESCE(SUM(1),0) FROM search_documents d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET bodies=(SELECT COALESCE(SUM((d.body_state='ready')::INTEGER),0) FROM search_documents d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET pending=(SELECT COALESCE(SUM((d.body_state='pending')::INTEGER),0) FROM search_documents d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET gaps=(SELECT COALESCE(SUM((d.body_state NOT IN ('pending','ready'))::INTEGER),0) FROM search_documents d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET body_bytes=(SELECT COALESCE(SUM((octet_length(d.body)+octet_length(d.attachment_text))),0) FROM search_documents d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET retrying=(SELECT COALESCE(SUM((d.body_state='fetch_error')::INTEGER),0) FROM search_documents d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET processed=(SELECT COALESCE(SUM((d.delivered=1 AND d.body_state NOT IN ('pending','fetch_error','quota'))::INTEGER),0) FROM search_documents d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET dirty=(SELECT COUNT(*) FROM search_dirty d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET delivery_pending=(SELECT COUNT(*) FROM search_index_outbox d WHERE d.account_id=t.account_id);
UPDATE search_totals t SET eligible=(SELECT COUNT(*) FROM search_inventory d WHERE d.account_id=t.account_id AND d.eligible);
