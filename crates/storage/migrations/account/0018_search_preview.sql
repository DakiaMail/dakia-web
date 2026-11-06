-- General search previews return bounded positives from all cached metadata. This is not
-- an FTS index and cannot certify complete provider search coverage.
CREATE INDEX messages_received_order ON messages(json_extract(document,'$.received_at') DESC, id DESC);
