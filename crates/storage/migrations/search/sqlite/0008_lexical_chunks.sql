-- Search projections are rebuildable. Keep raw documents and model vectors;
-- replace the lexical projection with bounded, backend-neutral token sequences.
DROP TRIGGER terms_insert;
DROP TRIGGER terms_delete;
DROP TRIGGER terms_update;
DROP TABLE terms;
ALTER TABLE documents ADD COLUMN lexical_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE embedding_receipts ADD COLUMN lexical_revision INTEGER NOT NULL DEFAULT 0;
CREATE TABLE lexical_chunks(
 id INTEGER PRIMARY KEY AUTOINCREMENT,account_id TEXT NOT NULL,message_id TEXT NOT NULL,
 content_version INTEGER NOT NULL,revision INTEGER NOT NULL,field INTEGER NOT NULL,
 ordinal INTEGER NOT NULL,text TEXT NOT NULL CHECK(length(CAST(text AS BLOB))<=65536),
 UNIQUE(account_id,message_id,field,ordinal),
 FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE
);
CREATE INDEX lexical_message ON lexical_chunks(account_id,message_id,content_version,revision);
CREATE VIRTUAL TABLE lexical_terms USING fts5(text,content=lexical_chunks,content_rowid=id,tokenize='unicode61 remove_diacritics 0');
INSERT INTO lexical_terms(lexical_terms,rank) VALUES('secure-delete',1);
CREATE TRIGGER lexical_chunk_insert AFTER INSERT ON lexical_chunks BEGIN INSERT INTO lexical_terms(rowid,text) VALUES(NEW.id,NEW.text);END;
CREATE TRIGGER lexical_chunk_delete AFTER DELETE ON lexical_chunks BEGIN INSERT INTO lexical_terms(lexical_terms,rowid,text) VALUES('delete',OLD.id,OLD.text);END;
CREATE TABLE lexical_accounts(account_id TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 2,pending INTEGER NOT NULL DEFAULT 0,cursor TEXT NOT NULL DEFAULT '',sweep_done INTEGER NOT NULL DEFAULT 0,clock INTEGER NOT NULL DEFAULT 0,retries INTEGER NOT NULL DEFAULT 0,retry_at INTEGER NOT NULL DEFAULT 0,error TEXT);
INSERT INTO lexical_accounts(account_id) SELECT account_id FROM account_progress;
CREATE TABLE lexical_jobs(queue_id INTEGER PRIMARY KEY AUTOINCREMENT,account_id TEXT NOT NULL,message_id TEXT NOT NULL,content_version INTEGER NOT NULL,revision INTEGER NOT NULL,field INTEGER NOT NULL DEFAULT 0,byte_offset INTEGER NOT NULL DEFAULT 0,carry TEXT NOT NULL DEFAULT '',skipping INTEGER NOT NULL DEFAULT 0,ordinal INTEGER NOT NULL DEFAULT 0,last_claim INTEGER NOT NULL DEFAULT 0,UNIQUE(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES documents(account_id,message_id) ON DELETE CASCADE);
CREATE INDEX lexical_priority ON lexical_jobs(account_id,queue_id DESC);
CREATE INDEX lexical_history ON lexical_jobs(account_id,last_claim,queue_id);
CREATE TRIGGER lexical_job_insert AFTER INSERT ON lexical_jobs BEGIN UPDATE lexical_accounts SET pending=pending+1 WHERE account_id=NEW.account_id;END;
CREATE TRIGGER lexical_job_delete AFTER DELETE ON lexical_jobs BEGIN UPDATE lexical_accounts SET pending=pending-1 WHERE account_id=OLD.account_id;END;
CREATE TRIGGER lexical_document_insert AFTER INSERT ON documents WHEN NEW.state<>'deleted' BEGIN
 INSERT INTO lexical_accounts(account_id) VALUES(NEW.account_id) ON CONFLICT DO NOTHING;
 INSERT INTO lexical_jobs(account_id,message_id,content_version,revision) SELECT NEW.account_id,NEW.message_id,NEW.content_version,revision FROM lexical_accounts WHERE account_id=NEW.account_id;
END;
CREATE TRIGGER lexical_document_update AFTER UPDATE OF content_version ON documents WHEN OLD.content_version<>NEW.content_version BEGIN
 DELETE FROM lexical_chunks WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 DELETE FROM lexical_jobs WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 UPDATE documents SET lexical_revision=0 WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO lexical_accounts(account_id) SELECT NEW.account_id WHERE NEW.state<>'deleted' ON CONFLICT DO NOTHING;
 INSERT INTO lexical_jobs(account_id,message_id,content_version,revision) SELECT NEW.account_id,NEW.message_id,NEW.content_version,revision FROM lexical_accounts WHERE account_id=NEW.account_id AND NEW.state<>'deleted';
END;
DROP TRIGGER embedding_receipt_update;
CREATE TRIGGER embedding_receipt_update AFTER UPDATE OF version,content_version,embedding_revision,embedding_state,state,lexical_revision ON documents BEGIN
 DELETE FROM embedding_receipts WHERE account_id=NEW.account_id AND message_id=NEW.message_id;
 INSERT INTO embedding_receipts(account_id,message_id,version,source,model_revision,content_version,lexical_revision) SELECT d.account_id,d.message_id,d.version,d.source,d.embedding_revision,d.content_version,d.lexical_revision FROM documents d JOIN lexical_accounts a ON a.account_id=d.account_id WHERE d.account_id=NEW.account_id AND d.message_id=NEW.message_id AND d.state NOT IN ('pending','fetch_error','quota','deleted') AND d.embedding_state IN ('ready','gap') AND d.embedding_revision<>'' AND d.lexical_revision=a.revision;
END;
UPDATE embedding_accounts SET sweep_cursor='',sweep_done=0;
