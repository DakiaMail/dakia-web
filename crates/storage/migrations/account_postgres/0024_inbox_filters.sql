CREATE INDEX messages_starred_received_order ON messages(account_id,received_at DESC,id DESC) WHERE starred AND cardinality(folder_ids)>0;
CREATE INDEX messages_attention_received_order ON messages(account_id,received_at DESC,id DESC) WHERE (document::jsonb#>>'{classification,attention_override}')::BOOLEAN IS TRUE OR jsonb_array_length(document::jsonb#>'{classification,attention}')>0;
CREATE INDEX messages_inbox_effective_kind ON messages(account_id,(COALESCE(document::jsonb#>>'{classification,kind}','personal')),received_at DESC,id DESC) WHERE upper_folders @> ARRAY['INBOX']::TEXT[];
CREATE TABLE message_types(account_id TEXT NOT NULL,message_id TEXT NOT NULL,email_type TEXT NOT NULL,received_at TEXT NOT NULL,PRIMARY KEY(account_id,message_id,email_type),FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE);
CREATE INDEX message_types_order ON message_types(account_id,email_type,received_at DESC,message_id DESC);
INSERT INTO message_types SELECT m.account_id,m.id,t,m.received_at FROM messages m CROSS JOIN LATERAL jsonb_array_elements_text(m.document::jsonb#>'{classification,types}') t ON CONFLICT DO NOTHING;
CREATE FUNCTION refresh_message_types() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN
 DELETE FROM message_types WHERE account_id=NEW.account_id AND message_id=NEW.id;
 INSERT INTO message_types SELECT NEW.account_id,NEW.id,t,NEW.received_at FROM jsonb_array_elements_text(NEW.document::jsonb#>'{classification,types}') t ON CONFLICT DO NOTHING;
 RETURN NEW;
END $$;
CREATE TRIGGER message_types_insert AFTER INSERT ON messages FOR EACH ROW EXECUTE FUNCTION refresh_message_types();
CREATE TRIGGER message_types_update AFTER UPDATE OF document ON messages FOR EACH ROW WHEN (OLD.document::jsonb#>'{classification,types}' IS DISTINCT FROM NEW.document::jsonb#>'{classification,types}' OR OLD.received_at IS DISTINCT FROM NEW.received_at) EXECUTE FUNCTION refresh_message_types();
