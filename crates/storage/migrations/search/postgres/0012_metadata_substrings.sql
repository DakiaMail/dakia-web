-- subject already has a pg_trgm GIN index. Escaped LIKE uses the same literal
-- semantics as normalized substring checks while allowing native index plans.
CREATE INDEX documents_sender_substrings ON documents USING gin(sender gin_trgm_ops);
CREATE INDEX documents_recipients_substrings ON documents USING gin(recipients gin_trgm_ops);
