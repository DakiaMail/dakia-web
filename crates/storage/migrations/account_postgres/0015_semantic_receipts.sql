ALTER TABLE search_documents ADD COLUMN delivered_version BIGINT NOT NULL DEFAULT 0;
CREATE INDEX search_delivery_upgrade ON search_documents(account_id,delivered_version,message_id);
CREATE TABLE search_semantic_profile(account_id TEXT PRIMARY KEY REFERENCES account_locks(account_id) ON DELETE CASCADE,model_revision TEXT NOT NULL,available BIGINT NOT NULL);
CREATE TABLE search_semantic_totals(account_id TEXT NOT NULL REFERENCES account_locks(account_id) ON DELETE CASCADE,model_revision TEXT NOT NULL,processed BIGINT NOT NULL DEFAULT 0,PRIMARY KEY(account_id,model_revision));
CREATE TABLE search_semantic_receipts(account_id TEXT NOT NULL,message_id TEXT NOT NULL,version BIGINT NOT NULL,source TEXT NOT NULL,model_revision TEXT NOT NULL,PRIMARY KEY(account_id,message_id),FOREIGN KEY(account_id,message_id) REFERENCES search_documents(account_id,message_id) ON DELETE CASCADE);
CREATE FUNCTION search_semantic_count() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN
 IF TG_OP<>'INSERT' THEN UPDATE search_semantic_totals SET processed=processed-1 WHERE account_id=OLD.account_id AND model_revision=OLD.model_revision;END IF;
 IF TG_OP<>'DELETE' THEN INSERT INTO search_semantic_totals(account_id,model_revision,processed) VALUES(NEW.account_id,NEW.model_revision,1) ON CONFLICT(account_id,model_revision) DO UPDATE SET processed=search_semantic_totals.processed+1;RETURN NEW;END IF;
 RETURN OLD;
END $$;
CREATE TRIGGER search_semantic_counts AFTER INSERT OR UPDATE OR DELETE ON search_semantic_receipts FOR EACH ROW EXECUTE FUNCTION search_semantic_count();
CREATE FUNCTION search_semantic_invalidate() RETURNS TRIGGER LANGUAGE plpgsql SET search_path=dakia_mail,pg_catalog AS $$
BEGIN DELETE FROM search_semantic_receipts WHERE account_id=NEW.account_id AND message_id=NEW.message_id;RETURN NEW;END $$;
CREATE TRIGGER search_semantic_dirty AFTER INSERT ON search_dirty FOR EACH ROW EXECUTE FUNCTION search_semantic_invalidate();
CREATE TRIGGER search_semantic_outbox AFTER INSERT OR UPDATE ON search_index_outbox FOR EACH ROW EXECUTE FUNCTION search_semantic_invalidate();
