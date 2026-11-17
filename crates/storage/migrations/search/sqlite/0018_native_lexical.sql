-- Preserve retained text, chunks and model vectors. The new projection is
-- populated by resumable lexical work; old processing proofs cannot claim it.
ALTER TABLE lexical_chunks ADD COLUMN native_text TEXT NOT NULL DEFAULT '';
ALTER TABLE lexical_chunks ADD COLUMN phrase_safe INTEGER NOT NULL DEFAULT 0;
DROP TRIGGER lexical_chunk_insert;
DROP TRIGGER lexical_chunk_delete;
DROP TABLE lexical_terms;
CREATE VIEW lexical_native_content AS SELECT id,native_text FROM lexical_chunks WHERE revision=4 AND native_text<>'';
CREATE VIRTUAL TABLE lexical_terms USING fts5(native_text,content=lexical_native_content,content_rowid=id,tokenize='unicode61 remove_diacritics 0');
INSERT INTO lexical_terms(lexical_terms,rank) VALUES('secure-delete',1);
CREATE TRIGGER lexical_chunk_insert AFTER INSERT ON lexical_chunks WHEN NEW.revision=4 AND NEW.native_text<>'' BEGIN
 INSERT INTO lexical_terms(rowid,native_text) VALUES(NEW.id,NEW.native_text);
END;
CREATE TRIGGER lexical_chunk_delete AFTER DELETE ON lexical_chunks WHEN OLD.revision=4 AND OLD.native_text<>'' BEGIN
 INSERT INTO lexical_terms(lexical_terms,rowid,native_text) VALUES('delete',OLD.id,OLD.native_text);
END;
CREATE TRIGGER lexical_chunk_update AFTER UPDATE OF native_text,revision ON lexical_chunks BEGIN
 INSERT INTO lexical_terms(lexical_terms,rowid,native_text) SELECT 'delete',OLD.id,OLD.native_text WHERE OLD.revision=4 AND OLD.native_text<>'';
 INSERT INTO lexical_terms(rowid,native_text) SELECT NEW.id,NEW.native_text WHERE NEW.revision=4 AND NEW.native_text<>'';
END;
CREATE TRIGGER spelling_chunk_update AFTER UPDATE OF text,revision ON lexical_chunks WHEN OLD.text<>NEW.text OR OLD.revision<3 BEGIN
 DELETE FROM spelling_chunk_words WHERE chunk_id=NEW.id;
 INSERT INTO spelling_chunk_words(chunk_id,account_id,word)
 SELECT DISTINCT NEW.id,NEW.account_id,substr(value,2) FROM json_each('["'||replace(NEW.text,' ','","')||'"]')
 WHERE substr(value,1,1)='w' AND length(value) BETWEEN 5 AND 33 AND value NOT GLOB '*[0-9]*';
END;
DROP TRIGGER spelling_lexical_account;
CREATE TRIGGER spelling_lexical_account AFTER INSERT ON lexical_accounts WHEN NEW.revision<4 BEGIN UPDATE lexical_accounts SET revision=4 WHERE account_id=NEW.account_id;END;
UPDATE lexical_accounts SET revision=4,cursor='',sweep_done=0,retries=0,retry_at=0,error=NULL;
UPDATE lexical_jobs SET revision=4,field=0,byte_offset=0,carry='',skipping=0,ordinal=0;
DELETE FROM embedding_receipts WHERE lexical_revision<>4;
CREATE INDEX lexical_chunk_lookup ON lexical_chunks(account_id,id,message_id,content_version,revision,field);
CREATE INDEX documents_lexical_score ON documents(account_id,message_id,content_version,state,subject,sender,recipients,participants);
