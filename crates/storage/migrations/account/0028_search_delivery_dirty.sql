-- A provisional deletion may have been delivered while metadata was dirty.
-- Even a no-op header update must publish a fresh receipt after reconciliation.
CREATE TRIGGER search_delivery_dirty AFTER DELETE ON search_dirty BEGIN
 UPDATE search_index_clock SET value=value+1 WHERE id=1;
 INSERT INTO search_index_outbox(message_id,sequence) SELECT old.message_id,value FROM search_index_clock WHERE id=1 ON CONFLICT(message_id) DO UPDATE SET sequence=excluded.sequence;
END;
