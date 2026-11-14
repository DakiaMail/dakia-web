-- Sparse views must not walk every recent message to find their first page.
CREATE INDEX messages_starred_received_order ON messages(json_extract(document,'$.received_at') DESC,id DESC)
WHERE json_extract(document,'$.starred')=1 AND json_array_length(document,'$.folder_ids')>0;
CREATE INDEX messages_attention_received_order ON messages(json_extract(document,'$.received_at') DESC,id DESC)
WHERE json_extract(document,'$.classification.attention_override')=1 OR json_array_length(document,'$.classification.attention')>0;
CREATE INDEX messages_thread_received_order ON messages(CAST(json_extract(document,'$.thread_id') AS TEXT),json_extract(document,'$.received_at') DESC,id DESC);
