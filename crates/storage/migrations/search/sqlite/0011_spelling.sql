-- Spelling data is derived from bounded canonical lexical chunks.
CREATE TABLE spelling_words(account_id TEXT NOT NULL,word TEXT NOT NULL,refs INTEGER NOT NULL CHECK(refs>0),PRIMARY KEY(account_id,word));
CREATE TABLE spelling_signatures(account_id TEXT NOT NULL,signature TEXT NOT NULL,word TEXT NOT NULL,PRIMARY KEY(account_id,signature,word),FOREIGN KEY(account_id,word) REFERENCES spelling_words(account_id,word) ON DELETE CASCADE);
CREATE TABLE spelling_chunk_words(chunk_id INTEGER NOT NULL,account_id TEXT NOT NULL,word TEXT NOT NULL,PRIMARY KEY(chunk_id,word),FOREIGN KEY(chunk_id) REFERENCES lexical_chunks(id) ON DELETE CASCADE,FOREIGN KEY(account_id,word) REFERENCES spelling_words(account_id,word) DEFERRABLE INITIALLY DEFERRED);
CREATE TRIGGER spelling_word_insert AFTER INSERT ON spelling_words BEGIN
 INSERT INTO spelling_signatures(account_id,signature,word)
 SELECT NEW.account_id,signature,NEW.word FROM (
  WITH RECURSIVE positions(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM positions WHERE i<length(NEW.word))
  SELECT CASE WHEN i=0 THEN NEW.word ELSE substr(NEW.word,1,i-1)||substr(NEW.word,i+1) END AS signature FROM positions
 ) GROUP BY signature;
END;
CREATE TRIGGER spelling_reference_insert AFTER INSERT ON spelling_chunk_words BEGIN
 INSERT INTO spelling_words VALUES(NEW.account_id,NEW.word,1) ON CONFLICT(account_id,word) DO UPDATE SET refs=refs+1;
END;
CREATE TRIGGER spelling_reference_delete AFTER DELETE ON spelling_chunk_words BEGIN
 DELETE FROM spelling_words WHERE account_id=OLD.account_id AND word=OLD.word AND refs=1;
 UPDATE spelling_words SET refs=refs-1 WHERE account_id=OLD.account_id AND word=OLD.word;
END;
CREATE TRIGGER spelling_chunk_insert AFTER INSERT ON lexical_chunks BEGIN
 INSERT INTO spelling_chunk_words(chunk_id,account_id,word)
 SELECT DISTINCT NEW.id,NEW.account_id,substr(value,2) FROM json_each('["'||replace(NEW.text,' ','","')||'"]')
 WHERE substr(value,1,1)='w' AND length(value) BETWEEN 5 AND 33 AND value NOT GLOB '*[0-9]*';
END;
-- Existing chunks are rebuilt gradually, without reacquiring text or embeddings.
CREATE TRIGGER spelling_lexical_account AFTER INSERT ON lexical_accounts WHEN NEW.revision=2 BEGIN UPDATE lexical_accounts SET revision=3 WHERE account_id=NEW.account_id;END;
UPDATE lexical_accounts SET revision=3,cursor='',sweep_done=0,retries=0,retry_at=0,error=NULL;
UPDATE lexical_jobs SET revision=3,field=0,byte_offset=0,carry='',skipping=0,ordinal=0;
