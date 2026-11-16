ALTER TABLE lexical_chunks ADD COLUMN native_text TEXT NOT NULL DEFAULT '';
ALTER TABLE lexical_chunks ADD COLUMN phrase_safe BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE lexical_chunks DROP COLUMN lexemes;
ALTER TABLE lexical_chunks ADD COLUMN lexemes TSVECTOR GENERATED ALWAYS AS (to_tsvector('simple'::regconfig,native_text)) STORED;
CREATE INDEX lexical_terms ON lexical_chunks USING gin(lexemes);
CREATE OR REPLACE FUNCTION spelling_chunk_updated() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_search,pg_catalog AS $$
BEGIN
 IF OLD.text=NEW.text AND OLD.revision>=3 THEN RETURN NEW;END IF;
 DELETE FROM spelling_chunk_words WHERE chunk_id=NEW.id;
 INSERT INTO spelling_chunk_words(chunk_id,account_id,word)
 SELECT DISTINCT NEW.id,NEW.account_id,substr(value,2) FROM unnest(string_to_array(NEW.text,' ')) value
 WHERE left(value,1)='w' AND length(value) BETWEEN 5 AND 33 AND value !~ '[0-9]';
 RETURN NEW;
END $$;
CREATE TRIGGER spelling_chunk_updated AFTER UPDATE OF text,revision ON lexical_chunks FOR EACH ROW EXECUTE FUNCTION spelling_chunk_updated();
ALTER TABLE lexical_accounts ALTER COLUMN revision SET DEFAULT 4;
UPDATE lexical_accounts SET revision=4,cursor='',sweep_done=0,retries=0,retry_at=0,error=NULL;
UPDATE lexical_jobs SET revision=4,field=0,byte_offset=0,carry='',skipping=0,ordinal=0;
DELETE FROM embedding_receipts WHERE lexical_revision<>4;
