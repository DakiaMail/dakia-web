-- Read mail stays unclassified until it becomes unread. Keep idle scheduling
-- proportional to eligible unread mail instead of rescanning the read archive.
CREATE INDEX messages_unread_classification
ON messages(json_extract(document,'$.received_at') DESC)
WHERE category_override IS NULL AND json_extract(document,'$.unread')=1
AND json_extract(document,'$.category_status')='pending';
