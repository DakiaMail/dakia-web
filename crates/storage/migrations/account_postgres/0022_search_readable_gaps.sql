-- Preserve available headers and bounded readable content on reacquisition.
-- Existing triggers revoke completion and publish the new acquisition state.
UPDATE search_documents SET body_state='pending',retry_at=0
WHERE body_state IN ('encrypted','oversized');
