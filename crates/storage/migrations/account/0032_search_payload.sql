-- Bodies are outbox payloads in independent-store mode, not a second permanent
-- text index. Legacy/offline FTS mode keeps its payloads as before.
ALTER TABLE search_documents ADD COLUMN body_payload INTEGER NOT NULL DEFAULT 0 CHECK(body_payload IN (0,1));
UPDATE search_documents SET body_payload=1;
DROP TRIGGER search_delivery_update;
CREATE TRIGGER search_delivery_update AFTER UPDATE OF source,sender,recipients,subject,received_at,body,body_state,participants,mime_recipients,attachment_text,attachment_details ON search_documents
WHEN NOT (old.body_payload=1 AND new.body_payload=0 AND old.source=new.source AND old.body_state=new.body_state AND old.sender=new.sender AND old.recipients=new.recipients AND old.subject=new.subject AND old.received_at=new.received_at) BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT new.message_id,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
