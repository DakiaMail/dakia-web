UPDATE folder_scans s SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=FALSE
WHERE EXISTS(SELECT 1 FROM messages m WHERE m.account_id=s.account_id AND m.document::jsonb->>'received_date' IS NULL);
