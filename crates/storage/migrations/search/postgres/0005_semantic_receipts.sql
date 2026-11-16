CREATE TABLE embedding_receipts(account_id TEXT NOT NULL,message_id TEXT NOT NULL,version BIGINT NOT NULL,source TEXT NOT NULL,model_revision TEXT NOT NULL,content_version BIGINT NOT NULL,PRIMARY KEY(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE);
CREATE INDEX embedding_receipts_order ON embedding_receipts(account_id,version,message_id);
CREATE FUNCTION embedding_completion_receipt() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 DELETE FROM embedding_receipts WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO embedding_receipts(account_id,message_id,version,source,model_revision,content_version) SELECT account_id,message_id,version,source,embedding_revision,content_version FROM documents WHERE account_id=NEW.account_id AND message_id=NEW.message_id AND state NOT IN ('pending','fetch_error','quota','deleted') AND embedding_state IN ('ready','gap') AND embedding_revision<>'';
 RETURN NEW;
END $$;
CREATE TRIGGER embedding_completion_receipt AFTER UPDATE OF version,content_version,embedding_revision,embedding_state,state ON documents FOR EACH ROW EXECUTE FUNCTION embedding_completion_receipt();
UPDATE embedding_accounts SET sweep_cursor='',sweep_done=0;
ALTER TABLE embedding_accounts ADD COLUMN retrying BIGINT NOT NULL DEFAULT 0;
UPDATE embedding_accounts a SET retrying=(SELECT count(*) FROM embedding_jobs j WHERE j.account_id=a.account_id AND j.state='retry');
CREATE FUNCTION embedding_retry_count() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP<>'INSERT' AND OLD.state='retry' THEN UPDATE embedding_accounts SET retrying=retrying-1 WHERE account_id=OLD.account_id;END IF;
 IF TG_OP<>'DELETE' AND NEW.state='retry' THEN UPDATE embedding_accounts SET retrying=retrying+1 WHERE account_id=NEW.account_id;END IF;
 IF TG_OP='DELETE' THEN RETURN OLD;END IF;RETURN NEW;
END $$;
CREATE TRIGGER embedding_retry_count AFTER INSERT OR UPDATE OF state OR DELETE ON embedding_jobs FOR EACH ROW EXECUTE FUNCTION embedding_retry_count();
