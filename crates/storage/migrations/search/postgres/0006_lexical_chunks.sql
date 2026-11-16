-- Remove only the derived single-document vector, not retained text or model
-- vectors. Its native position and size limits cannot represent long phrases.
ALTER TABLE documents DROP COLUMN lexemes;
ALTER TABLE documents ADD COLUMN lexical_revision BIGINT NOT NULL DEFAULT 0;
ALTER TABLE embedding_receipts ADD COLUMN lexical_revision BIGINT NOT NULL DEFAULT 0;
CREATE TABLE lexical_chunks(
 id BIGSERIAL PRIMARY KEY,account_id TEXT NOT NULL,message_id TEXT NOT NULL,
 content_version BIGINT NOT NULL,revision BIGINT NOT NULL,field BIGINT NOT NULL,
 ordinal BIGINT NOT NULL,text TEXT NOT NULL CHECK(octet_length(text)<=65536),
 lexemes tsvector GENERATED ALWAYS AS (strip(to_tsvector('simple'::regconfig,text))) STORED,
 UNIQUE(account_id,message_id,field,ordinal),
 FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE
);
CREATE INDEX lexical_message ON lexical_chunks(account_id,message_id,content_version,revision);
CREATE INDEX lexical_terms ON lexical_chunks USING gin(lexemes);
CREATE TABLE lexical_accounts(account_id TEXT PRIMARY KEY,revision BIGINT NOT NULL DEFAULT 2,pending BIGINT NOT NULL DEFAULT 0,cursor TEXT NOT NULL DEFAULT '',sweep_done BIGINT NOT NULL DEFAULT 0,clock BIGINT NOT NULL DEFAULT 0,retries BIGINT NOT NULL DEFAULT 0,retry_at BIGINT NOT NULL DEFAULT 0,error TEXT);
INSERT INTO lexical_accounts(account_id) SELECT account_id FROM account_progress;
CREATE TABLE lexical_jobs(queue_id BIGSERIAL PRIMARY KEY,account_id TEXT NOT NULL,message_id TEXT NOT NULL,content_version BIGINT NOT NULL,revision BIGINT NOT NULL,field BIGINT NOT NULL DEFAULT 0,byte_offset BIGINT NOT NULL DEFAULT 0,carry TEXT NOT NULL DEFAULT '',skipping BIGINT NOT NULL DEFAULT 0,ordinal BIGINT NOT NULL DEFAULT 0,last_claim BIGINT NOT NULL DEFAULT 0,UNIQUE(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE);
CREATE INDEX lexical_priority ON lexical_jobs(account_id,queue_id DESC);
CREATE INDEX lexical_history ON lexical_jobs(account_id,last_claim,queue_id);
CREATE FUNCTION lexical_job_count() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN UPDATE lexical_accounts SET pending=pending-1 WHERE account_id=OLD.account_id;RETURN OLD;END IF;
 UPDATE lexical_accounts SET pending=pending+1 WHERE account_id=NEW.account_id;RETURN NEW;
END $$;
CREATE TRIGGER lexical_job_count AFTER INSERT OR DELETE ON lexical_jobs FOR EACH ROW EXECUTE FUNCTION lexical_job_count();
CREATE FUNCTION lexical_document_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='UPDATE' AND OLD.content_version=NEW.content_version THEN RETURN NEW;END IF;
 IF TG_OP='UPDATE' THEN
  DELETE FROM lexical_chunks WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
  DELETE FROM lexical_jobs WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
  UPDATE documents SET lexical_revision=0 WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 END IF;
 IF NEW.state<>'deleted' THEN
  INSERT INTO lexical_accounts(account_id) VALUES(NEW.account_id) ON CONFLICT DO NOTHING;
  INSERT INTO lexical_jobs(account_id,message_id,content_version,revision) SELECT NEW.account_id,NEW.message_id,NEW.content_version,revision FROM lexical_accounts WHERE account_id=NEW.account_id;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER lexical_document_changed AFTER INSERT OR UPDATE OF content_version ON documents FOR EACH ROW EXECUTE FUNCTION lexical_document_changed();
CREATE OR REPLACE FUNCTION embedding_completion_receipt() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 DELETE FROM embedding_receipts WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO embedding_receipts(account_id,message_id,version,source,model_revision,content_version,lexical_revision) SELECT d.account_id,d.message_id,d.version,d.source,d.embedding_revision,d.content_version,d.lexical_revision FROM documents d JOIN lexical_accounts a ON a.account_id=d.account_id WHERE d.account_id=NEW.account_id AND d.message_id=NEW.message_id AND d.state NOT IN ('pending','fetch_error','quota','deleted') AND d.embedding_state IN ('ready','gap') AND d.embedding_revision<>'' AND d.lexical_revision=a.revision;
 RETURN NEW;
END $$;
DROP TRIGGER embedding_completion_receipt ON documents;
CREATE TRIGGER embedding_completion_receipt AFTER UPDATE OF version,content_version,embedding_revision,embedding_state,state,lexical_revision ON documents FOR EACH ROW EXECUTE FUNCTION embedding_completion_receipt();
UPDATE embedding_accounts SET sweep_cursor='',sweep_done=0;
