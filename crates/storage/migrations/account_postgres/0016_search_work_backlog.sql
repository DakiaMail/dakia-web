CREATE TABLE search_cleanup_work(account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,message_id TEXT NOT NULL,dirty BOOLEAN NOT NULL,outbox BOOLEAN NOT NULL,eligible BOOLEAN NOT NULL,PRIMARY KEY(account_id,message_id));
CREATE TABLE search_cleanup_totals(account_id TEXT PRIMARY KEY REFERENCES account_locks(account_id) ON DELETE CASCADE,pending BIGINT NOT NULL DEFAULT 0);
INSERT INTO search_cleanup_totals(account_id) SELECT account_id FROM account_locks;
CREATE FUNCTION search_cleanup_account() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$ BEGIN INSERT INTO search_cleanup_totals(account_id) VALUES(NEW.account_id);RETURN NEW;END $$;
CREATE TRIGGER search_cleanup_account AFTER INSERT ON account_locks FOR EACH ROW EXECUTE FUNCTION search_cleanup_account();
CREATE FUNCTION search_cleanup_count() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP<>'INSERT' THEN UPDATE search_cleanup_totals SET pending=pending-(NOT OLD.eligible AND (OLD.dirty OR OLD.outbox))::INTEGER WHERE account_id=OLD.account_id;END IF;
 IF TG_OP<>'DELETE' THEN UPDATE search_cleanup_totals SET pending=pending+(NOT NEW.eligible AND (NEW.dirty OR NEW.outbox))::INTEGER WHERE account_id=NEW.account_id;RETURN NEW;END IF;
 RETURN OLD;
END $$;
CREATE TRIGGER search_cleanup_count AFTER INSERT OR UPDATE OR DELETE ON search_cleanup_work FOR EACH ROW EXECUTE FUNCTION search_cleanup_count();
INSERT INTO search_cleanup_work SELECT p.account_id,p.id,EXISTS(SELECT 1 FROM search_dirty d WHERE d.account_id=p.account_id AND d.message_id=p.id),EXISTS(SELECT 1 FROM search_index_outbox o WHERE o.account_id=p.account_id AND o.message_id=p.id),EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=p.account_id AND e.id=p.id) FROM (SELECT account_id,message_id AS id FROM search_dirty UNION SELECT account_id,message_id FROM search_index_outbox) p;
CREATE FUNCTION search_cleanup_refresh(owner_account TEXT,identity TEXT) RETURNS VOID LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM account_locks WHERE account_id=owner_account) THEN RETURN;END IF;
 INSERT INTO search_cleanup_work(account_id,message_id,dirty,outbox,eligible) VALUES(owner_account,identity,EXISTS(SELECT 1 FROM search_dirty WHERE account_id=owner_account AND message_id=identity),EXISTS(SELECT 1 FROM search_index_outbox WHERE account_id=owner_account AND message_id=identity),EXISTS(SELECT 1 FROM search_eligible_messages WHERE account_id=owner_account AND id=identity)) ON CONFLICT(account_id,message_id) DO UPDATE SET dirty=excluded.dirty,outbox=excluded.outbox,eligible=excluded.eligible;
 DELETE FROM search_cleanup_work WHERE account_id=owner_account AND message_id=identity AND NOT dirty AND NOT outbox;
END $$;
CREATE FUNCTION search_cleanup_queue() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN PERFORM search_cleanup_refresh(OLD.account_id,OLD.message_id);RETURN OLD;END IF;
 PERFORM search_cleanup_refresh(NEW.account_id,NEW.message_id);RETURN NEW;
END $$;
CREATE TRIGGER search_cleanup_dirty AFTER INSERT OR DELETE ON search_dirty FOR EACH ROW EXECUTE FUNCTION search_cleanup_queue();
CREATE TRIGGER search_cleanup_outbox AFTER INSERT OR UPDATE OR DELETE ON search_index_outbox FOR EACH ROW EXECUTE FUNCTION search_cleanup_queue();
CREATE FUNCTION search_cleanup_message() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN UPDATE search_cleanup_work SET eligible=FALSE WHERE account_id=OLD.account_id AND message_id=OLD.id;RETURN OLD;END IF;
 UPDATE search_cleanup_work w SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=w.account_id AND e.id=w.message_id) WHERE w.account_id=NEW.account_id AND w.message_id=NEW.id;RETURN NEW;
END $$;
CREATE TRIGGER search_cleanup_message AFTER INSERT OR UPDATE OF document OR DELETE ON messages FOR EACH ROW EXECUTE FUNCTION search_cleanup_message();
CREATE FUNCTION search_cleanup_epoch() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
DECLARE owner_account TEXT;folder_name TEXT;
BEGIN
 IF TG_OP='DELETE' THEN owner_account=OLD.account_id;folder_name=substring(OLD.key FROM 14);ELSE owner_account=NEW.account_id;folder_name=substring(NEW.key FROM 14);END IF;
 UPDATE search_cleanup_work w SET eligible=EXISTS(SELECT 1 FROM search_eligible_messages e WHERE e.account_id=w.account_id AND e.id=w.message_id) FROM messages m WHERE w.account_id=owner_account AND m.account_id=w.account_id AND m.id=w.message_id AND m.provider_folder=folder_name;
 IF TG_OP='DELETE' THEN RETURN OLD;END IF;RETURN NEW;
END $$;
CREATE TRIGGER search_cleanup_epoch_insert AFTER INSERT ON provider_documents FOR EACH ROW WHEN(left(NEW.key,13)='thread:epoch:') EXECUTE FUNCTION search_cleanup_epoch();
CREATE TRIGGER search_cleanup_epoch_update AFTER UPDATE OF document ON provider_documents FOR EACH ROW WHEN(left(NEW.key,13)='thread:epoch:') EXECUTE FUNCTION search_cleanup_epoch();
CREATE TRIGGER search_cleanup_epoch_delete AFTER DELETE ON provider_documents FOR EACH ROW WHEN(left(OLD.key,13)='thread:epoch:') EXECUTE FUNCTION search_cleanup_epoch();
