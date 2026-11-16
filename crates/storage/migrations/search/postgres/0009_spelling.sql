CREATE TABLE spelling_words(account_id TEXT NOT NULL,word TEXT NOT NULL,refs BIGINT NOT NULL CHECK(refs>0),PRIMARY KEY(account_id,word));
CREATE TABLE spelling_signatures(account_id TEXT NOT NULL,signature TEXT NOT NULL,word TEXT NOT NULL,PRIMARY KEY(account_id,signature,word),FOREIGN KEY(account_id,word) REFERENCES spelling_words(account_id,word) ON DELETE CASCADE);
CREATE TABLE spelling_chunk_words(chunk_id BIGINT NOT NULL,account_id TEXT NOT NULL,word TEXT NOT NULL,PRIMARY KEY(chunk_id,word),FOREIGN KEY(chunk_id) REFERENCES lexical_chunks(id) ON DELETE CASCADE,FOREIGN KEY(account_id,word) REFERENCES spelling_words(account_id,word) DEFERRABLE INITIALLY DEFERRED);
CREATE FUNCTION spelling_word_insert() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 INSERT INTO spelling_signatures(account_id,signature,word)
 SELECT NEW.account_id,signature,NEW.word FROM (
  SELECT CASE WHEN i=0 THEN NEW.word ELSE substr(NEW.word,1,i-1)||substr(NEW.word,i+1) END AS signature FROM generate_series(0,length(NEW.word)) i
 ) s GROUP BY signature;
 RETURN NEW;
END $$;
CREATE TRIGGER spelling_word_insert AFTER INSERT ON spelling_words FOR EACH ROW EXECUTE FUNCTION spelling_word_insert();
CREATE FUNCTION spelling_reference_changed() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN
  DELETE FROM spelling_words WHERE account_id=OLD.account_id AND word=OLD.word AND refs=1;
  UPDATE spelling_words SET refs=refs-1 WHERE account_id=OLD.account_id AND word=OLD.word;
  RETURN OLD;
 END IF;
 INSERT INTO spelling_words VALUES(NEW.account_id,NEW.word,1) ON CONFLICT(account_id,word) DO UPDATE SET refs=spelling_words.refs+1;
 RETURN NEW;
END $$;
CREATE TRIGGER spelling_reference_changed AFTER INSERT OR DELETE ON spelling_chunk_words FOR EACH ROW EXECUTE FUNCTION spelling_reference_changed();
CREATE FUNCTION spelling_chunk_insert() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 INSERT INTO spelling_chunk_words(chunk_id,account_id,word)
 SELECT DISTINCT NEW.id,NEW.account_id,substr(value,2) FROM unnest(string_to_array(NEW.text,' ')) value
 WHERE left(value,1)='w' AND length(value) BETWEEN 5 AND 33 AND value !~ '[0-9]';
 RETURN NEW;
END $$;
CREATE TRIGGER spelling_chunk_insert AFTER INSERT ON lexical_chunks FOR EACH ROW EXECUTE FUNCTION spelling_chunk_insert();
ALTER TABLE lexical_accounts ALTER COLUMN revision SET DEFAULT 3;
UPDATE lexical_accounts SET revision=3,cursor='',sweep_done=0,retries=0,retry_at=0,error=NULL;
UPDATE lexical_jobs SET revision=3,field=0,byte_offset=0,carry='',skipping=0,ordinal=0;
