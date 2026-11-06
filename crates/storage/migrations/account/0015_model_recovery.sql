ALTER TABLE model_jobs ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE model_jobs ADD COLUMN retry_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE model_jobs ADD COLUMN failure_reason TEXT;
-- Keep this partial so classification's pending/running identity lookup still
-- uses model_jobs_active_message instead of scanning active jobs per message.
CREATE INDEX model_jobs_due ON model_jobs(retry_at) WHERE state='pending';

-- Older workers permanently failed classification even for temporary fetch
-- outages and did not retain a reason. Give those messages one fresh bounded
-- attempt on upgrade; never change an owner's manual category or active job.
INSERT INTO events(revision,kind,target_id)
SELECT revision+1,'message_changed',id FROM messages
WHERE category_override IS NULL AND json_extract(document,'$.category_status')='failed'
AND NOT EXISTS(SELECT 1 FROM model_jobs WHERE json_extract(model_jobs.document,'$.message_id')=messages.id AND state IN ('pending','running'));
UPDATE messages SET revision=revision+1,
 document=json_set(document,'$.revision',revision+1,'$.category_status','pending')
WHERE category_override IS NULL AND json_extract(document,'$.category_status')='failed'
AND NOT EXISTS(SELECT 1 FROM model_jobs WHERE json_extract(model_jobs.document,'$.message_id')=messages.id AND state IN ('pending','running'));
