-- A successful scan of the exposed folder is not proof of full coverage.
-- Keep authoritative completed_at exclusively for safe reconciliation/pruning.
ALTER TABLE folder_scans ADD COLUMN last_scanned_at INTEGER;
UPDATE folder_scans SET last_scanned_at=completed_at WHERE completed_at IS NOT NULL;
