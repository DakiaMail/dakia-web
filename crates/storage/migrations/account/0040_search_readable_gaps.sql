-- Older extraction discarded readable MIME headers when a body was encrypted
-- or too large. Reacquire these finite terminal gaps once with the new parser.
UPDATE search_documents SET body_state='pending',retry_at=0
WHERE body_state IN ('encrypted','oversized');
