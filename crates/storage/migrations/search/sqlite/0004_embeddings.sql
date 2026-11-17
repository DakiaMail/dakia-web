ALTER TABLE documents ADD COLUMN content_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE documents ADD COLUMN embedding_revision TEXT NOT NULL DEFAULT '';
ALTER TABLE documents ADD COLUMN embedding_state TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE documents ADD COLUMN embedding_error TEXT;
CREATE TABLE embedding_accounts (
 account_id TEXT PRIMARY KEY,model_revision TEXT NOT NULL,sweep_cursor TEXT NOT NULL DEFAULT '',
 sweep_done INTEGER NOT NULL DEFAULT 0,model_state TEXT NOT NULL DEFAULT 'ready',pending INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE embedding_progress (
 account_id TEXT NOT NULL,model_revision TEXT NOT NULL,complete INTEGER NOT NULL DEFAULT 0,gaps INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(account_id,model_revision)
);
-- Use non-reusable chunk identities for durable accelerator replay.
ALTER TABLE chunks RENAME TO old_chunks;
CREATE TABLE chunks (
 id INTEGER PRIMARY KEY AUTOINCREMENT,account_id TEXT NOT NULL,message_id TEXT NOT NULL,
 source TEXT NOT NULL,model_revision TEXT NOT NULL,ordinal INTEGER NOT NULL,text TEXT NOT NULL,
 embedding BLOB NOT NULL CHECK(length(embedding)=1536),content_version INTEGER NOT NULL DEFAULT 0,field INTEGER NOT NULL DEFAULT 0,
 byte_start INTEGER NOT NULL DEFAULT 0,byte_end INTEGER NOT NULL DEFAULT 0,
 UNIQUE(account_id,message_id,model_revision,ordinal),
 FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE
);
INSERT INTO chunks(id,account_id,message_id,source,model_revision,ordinal,text,embedding)
 SELECT id,account_id,message_id,source,model_revision,ordinal,text,embedding FROM old_chunks c
 WHERE length(c.embedding)=1536 AND EXISTS(SELECT 1 FROM documents d WHERE d.account_id=c.account_id AND d.message_id=c.message_id);
DROP TABLE old_chunks;
CREATE INDEX chunks_message ON chunks(account_id,message_id);
CREATE TABLE embedding_changes(seq INTEGER PRIMARY KEY AUTOINCREMENT,chunk_id INTEGER NOT NULL,account_id TEXT NOT NULL,model_revision TEXT NOT NULL);
INSERT INTO embedding_changes(chunk_id,account_id,model_revision) SELECT id,account_id,model_revision FROM chunks;
CREATE TRIGGER embedding_chunk_insert AFTER INSERT ON chunks BEGIN INSERT INTO embedding_changes(chunk_id,account_id,model_revision) VALUES(NEW.id,NEW.account_id,NEW.model_revision);END;
CREATE TRIGGER embedding_chunk_update AFTER UPDATE ON chunks BEGIN INSERT INTO embedding_changes(chunk_id,account_id,model_revision) VALUES(NEW.id,NEW.account_id,NEW.model_revision);END;
CREATE TRIGGER embedding_chunk_delete AFTER DELETE ON chunks BEGIN INSERT INTO embedding_changes(chunk_id,account_id,model_revision) VALUES(OLD.id,OLD.account_id,OLD.model_revision);END;
CREATE TABLE embedding_jobs (
 queue_id INTEGER PRIMARY KEY AUTOINCREMENT,account_id TEXT NOT NULL,message_id TEXT NOT NULL,
 model_revision TEXT NOT NULL,content_version INTEGER NOT NULL,field INTEGER NOT NULL DEFAULT 0,
 byte_offset INTEGER NOT NULL DEFAULT 0,ordinal INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL DEFAULT 'pending',
 lease_token TEXT,lease_until INTEGER NOT NULL DEFAULT 0,retry_at INTEGER NOT NULL DEFAULT 0,
 retries INTEGER NOT NULL DEFAULT 0,error TEXT,last_claim INTEGER NOT NULL DEFAULT 0,
 UNIQUE(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE
);
CREATE INDEX embedding_pending ON embedding_jobs(account_id,model_revision,state,queue_id);
CREATE INDEX embedding_fair_history ON embedding_jobs(account_id,model_revision,state,last_claim,queue_id);
CREATE TABLE embedding_clock(id INTEGER PRIMARY KEY CHECK(id=1),value INTEGER NOT NULL);
INSERT INTO embedding_clock VALUES(1,0);
CREATE INDEX embedding_retry ON embedding_jobs(account_id,state,retry_at);
CREATE INDEX embedding_leases ON embedding_jobs(account_id,state,lease_until);
CREATE TRIGGER embedding_content_version AFTER UPDATE OF source,sender,recipients,subject,body,participants,attachment_text,state ON documents WHEN OLD.source<>NEW.source OR OLD.sender<>NEW.sender OR OLD.recipients<>NEW.recipients OR OLD.subject<>NEW.subject OR OLD.body<>NEW.body OR OLD.participants<>NEW.participants OR OLD.attachment_text<>NEW.attachment_text OR ((OLD.state='deleted')<>(NEW.state='deleted')) BEGIN
 UPDATE documents SET content_version=OLD.content_version+1,embedding_revision='',embedding_state='pending',embedding_error=NULL WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
END;
CREATE TRIGGER embedding_enqueue_insert AFTER INSERT ON documents BEGIN INSERT INTO embedding_jobs(account_id,message_id,model_revision,content_version,state) SELECT NEW.account_id,NEW.message_id,COALESCE((SELECT model_revision FROM embedding_accounts WHERE account_id=NEW.account_id),''),NEW.content_version,CASE WHEN NEW.state IN ('pending','fetch_error','quota') THEN 'waiting' ELSE 'pending' END WHERE NEW.state<>'deleted' ON CONFLICT(account_id,message_id) DO UPDATE SET model_revision=excluded.model_revision,content_version=excluded.content_version,field=0,byte_offset=0,ordinal=0,state=excluded.state,lease_token=NULL,lease_until=0,retry_at=0,retries=0,error=NULL; END;
CREATE TRIGGER embedding_enqueue_update AFTER UPDATE OF content_version ON documents WHEN OLD.content_version<>NEW.content_version BEGIN
 DELETE FROM chunks WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 DELETE FROM embedding_jobs WHERE account_id=NEW.account_id AND message_id=NEW.message_id AND NEW.state='deleted';
 INSERT INTO embedding_jobs(account_id,message_id,model_revision,content_version,state) SELECT NEW.account_id,NEW.message_id,COALESCE((SELECT model_revision FROM embedding_accounts WHERE account_id=NEW.account_id),''),NEW.content_version,CASE WHEN NEW.state IN ('pending','fetch_error','quota') THEN 'waiting' ELSE 'pending' END WHERE NEW.state<>'deleted' ON CONFLICT(account_id,message_id) DO UPDATE SET model_revision=excluded.model_revision,content_version=excluded.content_version,field=0,byte_offset=0,ordinal=0,state=excluded.state,lease_token=NULL,lease_until=0,retry_at=0,retries=0,error=NULL;
END;
CREATE TRIGGER embedding_queue_insert AFTER INSERT ON embedding_jobs BEGIN UPDATE embedding_accounts SET pending=pending+1 WHERE account_id=NEW.account_id;END;
CREATE TRIGGER embedding_queue_delete AFTER DELETE ON embedding_jobs BEGIN UPDATE embedding_accounts SET pending=pending-1 WHERE account_id=OLD.account_id;END;
CREATE TRIGGER embedding_progress_insert AFTER INSERT ON documents WHEN NEW.embedding_revision<>'' BEGIN
 INSERT INTO embedding_progress(account_id,model_revision,complete,gaps) VALUES(NEW.account_id,NEW.embedding_revision,(NEW.embedding_state IN ('ready','gap')),(NEW.embedding_state='gap')) ON CONFLICT(account_id,model_revision) DO UPDATE SET complete=complete+excluded.complete,gaps=gaps+excluded.gaps;
END;
CREATE TRIGGER embedding_progress_delete AFTER DELETE ON documents WHEN OLD.embedding_revision<>'' BEGIN
 UPDATE embedding_progress SET complete=complete-(OLD.embedding_state IN ('ready','gap')),gaps=gaps-(OLD.embedding_state='gap') WHERE account_id=OLD.account_id AND model_revision=OLD.embedding_revision;
END;
CREATE TRIGGER embedding_progress_update AFTER UPDATE OF embedding_revision,embedding_state ON documents BEGIN
 UPDATE embedding_progress SET complete=complete-(OLD.embedding_state IN ('ready','gap')),gaps=gaps-(OLD.embedding_state='gap') WHERE account_id=OLD.account_id AND model_revision=OLD.embedding_revision;
 INSERT INTO embedding_progress(account_id,model_revision,complete,gaps) SELECT NEW.account_id,NEW.embedding_revision,(NEW.embedding_state IN ('ready','gap')),(NEW.embedding_state='gap') WHERE NEW.embedding_revision<>'' ON CONFLICT(account_id,model_revision) DO UPDATE SET complete=complete+excluded.complete,gaps=gaps+excluded.gaps;
END;

CREATE TRIGGER embedding_acquisition_state AFTER UPDATE OF state ON documents WHEN OLD.content_version=NEW.content_version AND (OLD.state IN ('pending','fetch_error','quota'))<>(NEW.state IN ('pending','fetch_error','quota')) BEGIN
 UPDATE embedding_jobs SET state=CASE WHEN NEW.state IN ('pending','fetch_error','quota') THEN 'waiting' ELSE 'pending' END,lease_token=NULL,lease_until=0 WHERE account_id=NEW.account_id AND message_id=NEW.message_id AND content_version=NEW.content_version;
END;
