-- Retain canonical text, embeddings and source identities. Field-specific
-- postings are populated by the existing resumable lexical worker.
DROP TRIGGER lexical_chunk_insert;
DROP TRIGGER lexical_chunk_delete;
DROP TRIGGER lexical_chunk_update;
DROP VIEW lexical_native_content;
CREATE VIEW lexical_native_content AS SELECT id,native_text FROM lexical_chunks WHERE revision IN (4,5) AND native_text<>'';
CREATE TRIGGER lexical_chunk_insert AFTER INSERT ON lexical_chunks WHEN NEW.revision IN (4,5) AND NEW.native_text<>'' BEGIN
 INSERT INTO lexical_terms(rowid,native_text) VALUES(NEW.id,NEW.native_text);
END;
CREATE TRIGGER lexical_chunk_delete AFTER DELETE ON lexical_chunks WHEN OLD.revision IN (4,5) AND OLD.native_text<>'' BEGIN
 INSERT INTO lexical_terms(lexical_terms,rowid,native_text) VALUES('delete',OLD.id,OLD.native_text);
END;
CREATE TRIGGER lexical_chunk_update AFTER UPDATE OF native_text,revision ON lexical_chunks BEGIN
 INSERT INTO lexical_terms(lexical_terms,rowid,native_text) SELECT 'delete',OLD.id,OLD.native_text WHERE OLD.revision IN (4,5) AND OLD.native_text<>'';
 INSERT INTO lexical_terms(rowid,native_text) SELECT NEW.id,NEW.native_text WHERE NEW.revision IN (4,5) AND NEW.native_text<>'';
END;
DROP TRIGGER spelling_lexical_account;
CREATE TRIGGER spelling_lexical_account AFTER INSERT ON lexical_accounts WHEN NEW.revision<5 BEGIN UPDATE lexical_accounts SET revision=5 WHERE account_id=NEW.account_id;END;
UPDATE lexical_accounts SET revision=5,cursor='',sweep_done=0,retries=0,retry_at=0,error=NULL;
UPDATE lexical_jobs SET revision=5,field=0,byte_offset=0,carry='',skipping=0,ordinal=0;
DELETE FROM embedding_receipts WHERE lexical_revision<>5;
