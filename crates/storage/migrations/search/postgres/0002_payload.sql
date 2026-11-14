ALTER TABLE documents ADD COLUMN metadata_recipients TEXT NOT NULL DEFAULT '';
ALTER TABLE documents ADD COLUMN mime_recipients TEXT NOT NULL DEFAULT '';
ALTER TABLE documents ADD COLUMN attachment_details TEXT NOT NULL DEFAULT '[]';
ALTER TABLE documents ADD COLUMN body_payload BIGINT NOT NULL DEFAULT 1;
UPDATE documents SET mime_recipients=recipients;
