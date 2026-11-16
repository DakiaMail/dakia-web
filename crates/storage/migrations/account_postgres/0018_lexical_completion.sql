ALTER TABLE search_semantic_receipts ADD COLUMN lexical_revision BIGINT NOT NULL DEFAULT 0;
ALTER TABLE search_semantic_profile ADD COLUMN lexical_revision BIGINT NOT NULL DEFAULT 2;
CREATE TABLE search_stage_totals(account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,model_revision TEXT NOT NULL,lexical_revision BIGINT NOT NULL,processed BIGINT NOT NULL DEFAULT 0,PRIMARY KEY(account_id,model_revision,lexical_revision));
CREATE INDEX search_legacy_stages ON search_semantic_receipts(account_id,message_id) WHERE lexical_revision<2;
CREATE FUNCTION search_stage_count() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP<>'INSERT' THEN UPDATE search_stage_totals SET processed=processed-1 WHERE account_id=OLD.account_id AND model_revision=OLD.model_revision AND lexical_revision=OLD.lexical_revision;END IF;
 IF TG_OP<>'DELETE' THEN INSERT INTO search_stage_totals(account_id,model_revision,lexical_revision,processed) VALUES(NEW.account_id,NEW.model_revision,NEW.lexical_revision,1) ON CONFLICT(account_id,model_revision,lexical_revision) DO UPDATE SET processed=search_stage_totals.processed+1;RETURN NEW;END IF;
 RETURN OLD;
END $$;
CREATE TRIGGER search_stage_count AFTER INSERT OR UPDATE OR DELETE ON search_semantic_receipts FOR EACH ROW EXECUTE FUNCTION search_stage_count();
INSERT INTO search_stage_totals SELECT account_id,model_revision,lexical_revision,count(*) FROM search_semantic_receipts GROUP BY account_id,model_revision,lexical_revision;
