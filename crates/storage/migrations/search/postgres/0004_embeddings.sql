ALTER TABLE documents ADD COLUMN content_version BIGINT NOT NULL DEFAULT 1;
ALTER TABLE documents ADD COLUMN embedding_revision TEXT NOT NULL DEFAULT '';
ALTER TABLE documents ADD COLUMN embedding_state TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE documents ADD COLUMN embedding_error TEXT;
CREATE TABLE embedding_accounts (
 account_id TEXT PRIMARY KEY,model_revision TEXT NOT NULL,sweep_cursor TEXT NOT NULL DEFAULT '',
 sweep_done BIGINT NOT NULL DEFAULT 0,model_state TEXT NOT NULL DEFAULT 'ready',pending BIGINT NOT NULL DEFAULT 0
);
CREATE TABLE embedding_progress (
 account_id TEXT NOT NULL,model_revision TEXT NOT NULL,complete BIGINT NOT NULL DEFAULT 0,gaps BIGINT NOT NULL DEFAULT 0,
 PRIMARY KEY(account_id,model_revision)
);
ALTER TABLE chunks ADD COLUMN content_version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN field BIGINT NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN byte_start BIGINT NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN byte_end BIGINT NOT NULL DEFAULT 0;
DELETE FROM chunks c WHERE NOT EXISTS(SELECT 1 FROM documents d WHERE d.account_id=c.account_id AND d.message_id=c.message_id);
ALTER TABLE chunks ADD FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE;
CREATE INDEX documents_embedding_sweep ON documents(account_id,message_id COLLATE "C");
CREATE TABLE embedding_jobs (
 queue_id BIGSERIAL PRIMARY KEY,account_id TEXT NOT NULL,message_id TEXT NOT NULL,
 model_revision TEXT NOT NULL,content_version BIGINT NOT NULL,field BIGINT NOT NULL DEFAULT 0,
 byte_offset BIGINT NOT NULL DEFAULT 0,ordinal BIGINT NOT NULL DEFAULT 0,state TEXT NOT NULL DEFAULT 'pending',
 lease_token TEXT,lease_until BIGINT NOT NULL DEFAULT 0,retry_at BIGINT NOT NULL DEFAULT 0,
 retries BIGINT NOT NULL DEFAULT 0,error TEXT,last_claim BIGINT NOT NULL DEFAULT 0,
 UNIQUE(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE
);
CREATE INDEX embedding_pending ON embedding_jobs(account_id,model_revision,state,queue_id);
CREATE INDEX embedding_fair_history ON embedding_jobs(account_id,model_revision,state,last_claim,queue_id);
CREATE TABLE embedding_clock(id BIGINT PRIMARY KEY CHECK(id=1),value BIGINT NOT NULL);
INSERT INTO embedding_clock VALUES(1,0);
CREATE INDEX embedding_retry ON embedding_jobs(account_id,state,retry_at);
CREATE INDEX embedding_leases ON embedding_jobs(account_id,state,lease_until);
-- An AFTER trigger keeps metadata-only writes out of PostgreSQL's stored
-- generated lexeme recomputation path. The nested update touches stage columns.
CREATE FUNCTION embedding_content_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 UPDATE documents SET content_version=OLD.content_version+1,embedding_revision='',embedding_state='pending',embedding_error=NULL WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 RETURN NULL;
END $$;
CREATE TRIGGER embedding_content_version AFTER UPDATE OF source,sender,recipients,subject,body,participants,attachment_text,state ON documents FOR EACH ROW WHEN (OLD.source IS DISTINCT FROM NEW.source OR OLD.sender IS DISTINCT FROM NEW.sender OR OLD.recipients IS DISTINCT FROM NEW.recipients OR OLD.subject IS DISTINCT FROM NEW.subject OR OLD.body IS DISTINCT FROM NEW.body OR OLD.participants IS DISTINCT FROM NEW.participants OR OLD.attachment_text IS DISTINCT FROM NEW.attachment_text OR ((OLD.state='deleted')<>(NEW.state='deleted'))) EXECUTE FUNCTION embedding_content_changed();
CREATE FUNCTION embedding_enqueue() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='UPDATE' AND OLD.content_version=NEW.content_version THEN
  IF (OLD.state IN ('pending','fetch_error','quota'))<>(NEW.state IN ('pending','fetch_error','quota')) THEN
   UPDATE embedding_jobs SET state=CASE WHEN NEW.state IN ('pending','fetch_error','quota') THEN 'waiting' ELSE 'pending' END,lease_token=NULL,lease_until=0 WHERE account_id=NEW.account_id AND message_id=NEW.message_id AND content_version=NEW.content_version;
  END IF;
  RETURN NEW;
 END IF;
 DELETE FROM chunks WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 DELETE FROM embedding_jobs WHERE account_id=NEW.account_id AND message_id=NEW.message_id AND NEW.state='deleted';
 INSERT INTO embedding_jobs(account_id,message_id,model_revision,content_version,state) SELECT NEW.account_id,NEW.message_id,COALESCE((SELECT model_revision FROM embedding_accounts WHERE account_id=NEW.account_id),''),NEW.content_version,CASE WHEN NEW.state IN ('pending','fetch_error','quota') THEN 'waiting' ELSE 'pending' END WHERE NEW.state<>'deleted' ON CONFLICT(account_id,message_id) DO UPDATE SET model_revision=excluded.model_revision,content_version=excluded.content_version,field=0,byte_offset=0,ordinal=0,state=excluded.state,lease_token=NULL,lease_until=0,retry_at=0,retries=0,error=NULL;
 RETURN NEW;
END $$;
CREATE TRIGGER embedding_enqueue AFTER INSERT OR UPDATE ON documents FOR EACH ROW EXECUTE FUNCTION embedding_enqueue();
CREATE FUNCTION embedding_queue_counts() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN UPDATE embedding_accounts SET pending=pending-1 WHERE account_id=OLD.account_id;RETURN OLD; END IF;
 UPDATE embedding_accounts SET pending=pending+1 WHERE account_id=NEW.account_id;RETURN NEW;
END $$;
CREATE TRIGGER embedding_queue_insert AFTER INSERT ON embedding_jobs FOR EACH ROW EXECUTE FUNCTION embedding_queue_counts();
CREATE TRIGGER embedding_queue_delete AFTER DELETE ON embedding_jobs FOR EACH ROW EXECUTE FUNCTION embedding_queue_counts();
CREATE FUNCTION embedding_progress_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='UPDATE' AND OLD.embedding_revision IS NOT DISTINCT FROM NEW.embedding_revision AND OLD.embedding_state IS NOT DISTINCT FROM NEW.embedding_state THEN RETURN NULL;END IF;
 IF TG_OP<>'INSERT' AND OLD.embedding_revision<>'' THEN
  UPDATE embedding_progress SET complete=complete-(OLD.embedding_state IN ('ready','gap'))::INTEGER,gaps=gaps-(OLD.embedding_state='gap')::INTEGER WHERE account_id=OLD.account_id AND model_revision=OLD.embedding_revision;
 END IF;
 IF TG_OP<>'DELETE' AND NEW.embedding_revision<>'' THEN
  INSERT INTO embedding_progress(account_id,model_revision,complete,gaps) VALUES(NEW.account_id,NEW.embedding_revision,(NEW.embedding_state IN ('ready','gap'))::INTEGER,(NEW.embedding_state='gap')::INTEGER) ON CONFLICT(account_id,model_revision) DO UPDATE SET complete=embedding_progress.complete+excluded.complete,gaps=embedding_progress.gaps+excluded.gaps;
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER embedding_progress AFTER INSERT OR UPDATE OR DELETE ON documents FOR EACH ROW EXECUTE FUNCTION embedding_progress_changed();
