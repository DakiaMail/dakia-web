CREATE FUNCTION search_snooze_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
DECLARE owner_account TEXT; thread TEXT;
BEGIN
 IF TG_OP='DELETE' THEN owner_account=OLD.account_id;thread=OLD.thread_id;ELSE owner_account=NEW.account_id;thread=NEW.thread_id;END IF;
 UPDATE account_locks SET search_sequence=search_sequence+1 WHERE account_id=owner_account;
 INSERT INTO search_index_outbox(account_id,message_id,sequence) SELECT m.account_id,m.id,c.search_sequence FROM messages m JOIN account_locks c ON c.account_id=m.account_id WHERE m.account_id=owner_account AND m.thread_id=thread ON CONFLICT(account_id,message_id) DO UPDATE SET sequence=excluded.sequence;
 IF TG_OP='DELETE' THEN RETURN OLD;END IF;RETURN NEW;
END $$;
CREATE TRIGGER search_snooze_changed AFTER INSERT OR UPDATE OF until OR DELETE ON snoozes FOR EACH ROW EXECUTE FUNCTION search_snooze_changed();
UPDATE account_locks c SET search_sequence=search_sequence+1 WHERE EXISTS(SELECT 1 FROM snoozes s WHERE s.account_id=c.account_id);
INSERT INTO search_index_outbox(account_id,message_id,sequence) SELECT m.account_id,m.id,c.search_sequence FROM messages m JOIN account_locks c ON c.account_id=m.account_id WHERE EXISTS(SELECT 1 FROM snoozes s WHERE s.account_id=m.account_id AND s.thread_id=m.thread_id) ON CONFLICT(account_id,message_id) DO UPDATE SET sequence=excluded.sequence;
