CREATE TABLE account_progress (account_id TEXT PRIMARY KEY,headers BIGINT NOT NULL DEFAULT 0,bodies BIGINT NOT NULL DEFAULT 0,pending BIGINT NOT NULL DEFAULT 0,gaps BIGINT NOT NULL DEFAULT 0,body_bytes BIGINT NOT NULL DEFAULT 0,retrying BIGINT NOT NULL DEFAULT 0);
INSERT INTO account_progress(account_id,headers,bodies,pending,gaps,body_bytes,retrying) SELECT account_id,SUM((state<>'deleted')::INTEGER),SUM((state='ready')::INTEGER),SUM((state='pending')::INTEGER),SUM((state NOT IN ('pending','ready','deleted'))::INTEGER),SUM((octet_length(body)+octet_length(attachment_text))),SUM((state='fetch_error')::INTEGER) FROM documents GROUP BY account_id;
CREATE FUNCTION document_progress() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='INSERT' THEN
 INSERT INTO account_progress(account_id,headers,bodies,pending,gaps,body_bytes,retrying) VALUES(NEW.account_id,(NEW.state<>'deleted')::INTEGER,(NEW.state='ready')::INTEGER,(NEW.state='pending')::INTEGER,(NEW.state NOT IN ('pending','ready','deleted'))::INTEGER,(octet_length(NEW.body)+octet_length(NEW.attachment_text)),(NEW.state='fetch_error')::INTEGER) ON CONFLICT(account_id) DO UPDATE SET headers=account_progress.headers+excluded.headers,bodies=account_progress.bodies+excluded.bodies,pending=account_progress.pending+excluded.pending,gaps=account_progress.gaps+excluded.gaps,body_bytes=account_progress.body_bytes+excluded.body_bytes,retrying=account_progress.retrying+excluded.retrying;
 RETURN NEW;
 END IF;
 IF TG_OP='DELETE' THEN
 UPDATE account_progress SET headers=headers-(OLD.state<>'deleted')::INTEGER,bodies=bodies-(OLD.state='ready')::INTEGER,pending=pending-(OLD.state='pending')::INTEGER,gaps=gaps-(OLD.state NOT IN ('pending','ready','deleted'))::INTEGER,body_bytes=body_bytes-(octet_length(OLD.body)+octet_length(OLD.attachment_text)),retrying=retrying-(OLD.state='fetch_error')::INTEGER WHERE account_id=OLD.account_id;
 RETURN OLD;
 END IF;
 UPDATE account_progress SET headers=headers+(NEW.state<>'deleted')::INTEGER-(OLD.state<>'deleted')::INTEGER,bodies=bodies+(NEW.state='ready')::INTEGER-(OLD.state='ready')::INTEGER,pending=pending+(NEW.state='pending')::INTEGER-(OLD.state='pending')::INTEGER,gaps=gaps+(NEW.state NOT IN ('pending','ready','deleted'))::INTEGER-(OLD.state NOT IN ('pending','ready','deleted'))::INTEGER,body_bytes=body_bytes+(octet_length(NEW.body)+octet_length(NEW.attachment_text))-(octet_length(OLD.body)+octet_length(OLD.attachment_text)),retrying=retrying+(NEW.state='fetch_error')::INTEGER-(OLD.state='fetch_error')::INTEGER WHERE account_id=NEW.account_id;
 RETURN NEW;
END $$;
CREATE TRIGGER document_progress AFTER INSERT OR UPDATE OR DELETE ON documents FOR EACH ROW EXECUTE FUNCTION document_progress();
