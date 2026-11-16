ALTER TABLE search_documents ADD COLUMN participants TEXT NOT NULL DEFAULT '';
ALTER TABLE search_documents ADD COLUMN mime_recipients TEXT NOT NULL DEFAULT '';
ALTER TABLE search_documents ADD COLUMN attachment_text TEXT NOT NULL DEFAULT '';
ALTER TABLE search_documents ADD COLUMN attachment_details TEXT NOT NULL DEFAULT '[]';
-- Re-extract content once to acquire participants and attachment text.
UPDATE search_documents SET body_state='pending',retry_at=0 WHERE body_state IN ('ready','parse_error','oversized');
