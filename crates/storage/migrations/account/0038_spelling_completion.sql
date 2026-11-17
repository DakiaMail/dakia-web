UPDATE search_semantic_profile SET lexical_revision=3;
CREATE INDEX search_spelling_stages ON search_semantic_receipts(message_id) WHERE lexical_revision<3;
