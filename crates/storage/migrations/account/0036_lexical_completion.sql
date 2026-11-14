ALTER TABLE search_semantic_receipts ADD COLUMN lexical_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_semantic_profile ADD COLUMN lexical_revision INTEGER NOT NULL DEFAULT 2;
CREATE TABLE search_stage_totals(model_revision TEXT NOT NULL,lexical_revision INTEGER NOT NULL,processed INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(model_revision,lexical_revision));
CREATE INDEX search_legacy_stages ON search_semantic_receipts(message_id) WHERE lexical_revision<2;
CREATE TRIGGER search_stage_insert AFTER INSERT ON search_semantic_receipts BEGIN INSERT INTO search_stage_totals VALUES(NEW.model_revision,NEW.lexical_revision,1) ON CONFLICT(model_revision,lexical_revision) DO UPDATE SET processed=processed+1;END;
CREATE TRIGGER search_stage_delete AFTER DELETE ON search_semantic_receipts BEGIN UPDATE search_stage_totals SET processed=processed-1 WHERE model_revision=OLD.model_revision AND lexical_revision=OLD.lexical_revision;END;
CREATE TRIGGER search_stage_update AFTER UPDATE ON search_semantic_receipts BEGIN
 UPDATE search_stage_totals SET processed=processed-1 WHERE model_revision=OLD.model_revision AND lexical_revision=OLD.lexical_revision;
 INSERT INTO search_stage_totals VALUES(NEW.model_revision,NEW.lexical_revision,1) ON CONFLICT(model_revision,lexical_revision) DO UPDATE SET processed=processed+1;
END;
INSERT INTO search_stage_totals SELECT model_revision,lexical_revision,count(*) FROM search_semantic_receipts GROUP BY model_revision,lexical_revision;
