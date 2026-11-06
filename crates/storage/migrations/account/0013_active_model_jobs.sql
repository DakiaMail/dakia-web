-- Classification shares the single account connection with sync evidence.
-- Never rescan completed job history for every pending message. Keep exactly
-- the existing active-job predicate, including running translation jobs.
CREATE INDEX model_jobs_active_message
ON model_jobs(json_extract(document,'$.message_id'))
WHERE state IN ('pending','running');
