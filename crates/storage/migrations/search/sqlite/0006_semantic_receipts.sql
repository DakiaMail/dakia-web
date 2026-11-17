CREATE TABLE embedding_receipts(account_id TEXT NOT NULL,message_id TEXT NOT NULL,version INTEGER NOT NULL,source TEXT NOT NULL,model_revision TEXT NOT NULL,content_version INTEGER NOT NULL,PRIMARY KEY(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE);
CREATE INDEX embedding_receipts_order ON embedding_receipts(account_id,version,message_id);
CREATE TRIGGER embedding_receipt_update AFTER UPDATE OF version,content_version,embedding_revision,embedding_state,state ON documents BEGIN
 DELETE FROM embedding_receipts WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO embedding_receipts(account_id,message_id,version,source,model_revision,content_version) SELECT account_id,message_id,version,source,embedding_revision,content_version FROM documents WHERE account_id=NEW.account_id AND message_id=NEW.message_id AND state NOT IN ('pending','fetch_error','quota','deleted') AND embedding_state IN ('ready','gap') AND embedding_revision<>'';
END;
-- Discover existing completed rows in bounded turns instead of migrating the
-- entire vector index in one transaction.
UPDATE embedding_accounts SET sweep_cursor='',sweep_done=0;
ALTER TABLE embedding_accounts ADD COLUMN retrying INTEGER NOT NULL DEFAULT 0;
UPDATE embedding_accounts SET retrying=(SELECT count(*) FROM embedding_jobs j WHERE j.account_id=embedding_accounts.account_id AND j.state='retry');
CREATE TRIGGER embedding_retry_insert AFTER INSERT ON embedding_jobs WHEN NEW.state='retry' BEGIN UPDATE embedding_accounts SET retrying=retrying+1 WHERE account_id=NEW.account_id;END;
CREATE TRIGGER embedding_retry_delete AFTER DELETE ON embedding_jobs WHEN OLD.state='retry' BEGIN UPDATE embedding_accounts SET retrying=retrying-1 WHERE account_id=OLD.account_id;END;
CREATE TRIGGER embedding_retry_update AFTER UPDATE OF state ON embedding_jobs BEGIN UPDATE embedding_accounts SET retrying=retrying+(NEW.state='retry')-(OLD.state='retry') WHERE account_id=NEW.account_id;END;
