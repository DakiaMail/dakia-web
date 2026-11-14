-- Keep last-reference removal bounded by the affected word, not the mailbox.
CREATE INDEX spelling_references_word ON spelling_chunk_words(account_id,word);
CREATE INDEX spelling_signatures_word ON spelling_signatures(account_id,word);
