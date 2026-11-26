ALTER TABLE messages ADD COLUMN sender_key TEXT;
DROP INDEX messages_sender_id;
CREATE INDEX messages_sender_key_id ON messages(sender_key, id);
