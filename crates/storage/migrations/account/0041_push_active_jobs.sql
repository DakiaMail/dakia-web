CREATE INDEX push_jobs_active ON push_jobs(state)
WHERE state IN ('pending','running');
