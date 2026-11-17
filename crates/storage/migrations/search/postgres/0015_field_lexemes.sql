-- Existing retained text/model vectors remain valid. The account sweep and
-- versioned completion receipts publish the field-specific lexical projection.
ALTER TABLE lexical_accounts ALTER COLUMN revision SET DEFAULT 5;
UPDATE lexical_accounts SET revision=5,cursor='',sweep_done=0,retries=0,retry_at=0,error=NULL;
UPDATE lexical_jobs SET revision=5,field=0,byte_offset=0,carry='',skipping=0,ordinal=0;
DELETE FROM embedding_receipts WHERE lexical_revision<>5;
