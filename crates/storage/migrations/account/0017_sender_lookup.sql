-- Exact address lookup for the immediate sender-cleanup catalogue.
CREATE INDEX messages_sender_id ON messages(json_extract(document,'$.sender.email'), id);
