-- Exact progress denominators come from eligible mailbox metadata, not just
-- headers the indexer has already reached. All counters are maintained on writes.
CREATE VIEW search_eligible_messages AS SELECT m.id FROM messages m WHERE json_array_length(m.document,'$.folder_ids')>0 AND NOT EXISTS(SELECT 1 FROM json_each(m.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(m.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END;
ALTER TABLE search_totals ADD COLUMN eligible INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_totals ADD COLUMN processed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_documents ADD COLUMN delivered INTEGER NOT NULL DEFAULT 0;
UPDATE search_totals SET eligible=(SELECT count(*) FROM search_eligible_messages);
DROP TRIGGER search_delivery_update;
CREATE TRIGGER search_delivery_update AFTER UPDATE OF source,sender,recipients,subject,received_at,body,body_state,participants,mime_recipients,attachment_text,attachment_details ON search_documents BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT new.message_id,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
CREATE TRIGGER search_progress_receipt_insert AFTER INSERT ON search_index_outbox BEGIN
 UPDATE search_documents SET delivered=0 WHERE message_id=new.message_id;
END;
CREATE TRIGGER search_progress_receipt_update AFTER UPDATE ON search_index_outbox BEGIN
 UPDATE search_documents SET delivered=0 WHERE message_id=new.message_id;
END;
CREATE TRIGGER search_processed_insert AFTER INSERT ON search_documents BEGIN
 UPDATE search_totals SET processed=processed+(new.delivered=1 AND new.body_state NOT IN ('pending','fetch_error','quota')) WHERE id=1;
END;
CREATE TRIGGER search_processed_delete AFTER DELETE ON search_documents BEGIN
 UPDATE search_totals SET processed=processed-(old.delivered=1 AND old.body_state NOT IN ('pending','fetch_error','quota')) WHERE id=1;
END;
CREATE TRIGGER search_processed_update AFTER UPDATE OF body_state,delivered ON search_documents BEGIN
 UPDATE search_totals SET processed=processed+(new.delivered=1 AND new.body_state NOT IN ('pending','fetch_error','quota'))-(old.delivered=1 AND old.body_state NOT IN ('pending','fetch_error','quota')) WHERE id=1;
END;
CREATE TRIGGER search_eligible_insert AFTER INSERT ON messages BEGIN
 UPDATE search_totals SET eligible=eligible+(json_array_length(new.document,'$.folder_ids')>0 AND NOT EXISTS(SELECT 1 FROM json_each(new.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) AND CASE WHEN json_valid(json_extract(new.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(new.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(new.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END) WHERE id=1;
END;
CREATE TRIGGER search_eligible_delete AFTER DELETE ON messages BEGIN
 UPDATE search_totals SET eligible=eligible-(json_array_length(old.document,'$.folder_ids')>0 AND NOT EXISTS(SELECT 1 FROM json_each(old.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) AND CASE WHEN json_valid(json_extract(old.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(old.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(old.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END) WHERE id=1;
END;
CREATE TRIGGER search_eligible_update AFTER UPDATE OF document ON messages BEGIN
 UPDATE search_totals SET eligible=eligible+(json_array_length(new.document,'$.folder_ids')>0 AND NOT EXISTS(SELECT 1 FROM json_each(new.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) AND CASE WHEN json_valid(json_extract(new.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(new.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(new.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END)-(json_array_length(old.document,'$.folder_ids')>0 AND NOT EXISTS(SELECT 1 FROM json_each(old.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) AND CASE WHEN json_valid(json_extract(old.document,'$.provider_id')) THEN NOT EXISTS(SELECT 1 FROM provider_documents v WHERE v.key='thread:epoch:'||json_extract(json_extract(old.document,'$.provider_id'),'$.folder') AND CAST(v.document AS INTEGER)<>json_extract(json_extract(old.document,'$.provider_id'),'$.uidvalidity')) ELSE 1 END) WHERE id=1;
END;
CREATE TRIGGER search_eligible_epoch_insert AFTER INSERT ON provider_documents WHEN new.key GLOB 'thread:epoch:*' BEGIN
 UPDATE search_totals SET eligible=(SELECT count(*) FROM search_eligible_messages) WHERE id=1;
 INSERT INTO search_dirty(message_id) SELECT m.id FROM messages m WHERE CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN json_extract(json_extract(m.document,'$.provider_id'),'$.folder') END=substr(new.key,14) AND NOT EXISTS(SELECT 1 FROM search_dirty d WHERE d.message_id=m.id);
 DELETE FROM search_documents WHERE message_id IN (SELECT m.id FROM messages m WHERE CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN json_extract(json_extract(m.document,'$.provider_id'),'$.folder') END=substr(new.key,14) AND CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN json_extract(json_extract(m.document,'$.provider_id'),'$.uidvalidity') END<>CAST(new.document AS INTEGER));
END;
CREATE TRIGGER search_eligible_epoch_update AFTER UPDATE OF document ON provider_documents WHEN new.key GLOB 'thread:epoch:*' AND old.document<>new.document BEGIN
 UPDATE search_totals SET eligible=(SELECT count(*) FROM search_eligible_messages) WHERE id=1;
 INSERT INTO search_dirty(message_id) SELECT m.id FROM messages m WHERE CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN json_extract(json_extract(m.document,'$.provider_id'),'$.folder') END=substr(new.key,14) AND NOT EXISTS(SELECT 1 FROM search_dirty d WHERE d.message_id=m.id);
END;
CREATE TRIGGER search_eligible_epoch_delete AFTER DELETE ON provider_documents WHEN old.key GLOB 'thread:epoch:*' BEGIN
 UPDATE search_totals SET eligible=(SELECT count(*) FROM search_eligible_messages) WHERE id=1;
 INSERT INTO search_dirty(message_id) SELECT m.id FROM messages m WHERE CASE WHEN json_valid(json_extract(m.document,'$.provider_id')) THEN json_extract(json_extract(m.document,'$.provider_id'),'$.folder') END=substr(old.key,14) AND NOT EXISTS(SELECT 1 FROM search_dirty d WHERE d.message_id=m.id);
END;
-- Existing deliveries must be observed once before claiming destination completion.
INSERT INTO search_index_outbox(message_id,sequence) SELECT message_id,(SELECT value+1 FROM search_index_clock WHERE id=1) FROM search_documents WHERE 1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
UPDATE search_index_clock SET value=value+1 WHERE id=1;
