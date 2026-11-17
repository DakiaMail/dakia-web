-- Old UTC timestamps cannot recover the provider calendar day. Revisit metadata
-- through the existing gradual scanner, retaining boundaries and arrival proofs.
UPDATE folder_scans SET generation=generation+1,continuation=NULL,completed_at=NULL,coverage_limited=0
WHERE EXISTS(SELECT 1 FROM messages WHERE json_extract(document,'$.received_date') IS NULL);
