UPDATE search_semantic_profile SET lexical_revision=3;
ALTER TABLE search_semantic_profile ALTER COLUMN lexical_revision SET DEFAULT 3;
CREATE INDEX search_spelling_stages ON search_semantic_receipts(account_id,message_id) WHERE lexical_revision<3;
