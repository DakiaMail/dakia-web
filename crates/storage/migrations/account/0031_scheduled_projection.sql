-- Unsubscribe submissions use an opaque request receipt rather than JSON.
-- Keep the Scheduled view independent of either request representation.
ALTER TABLE submissions ADD COLUMN scheduled_by_user INTEGER NOT NULL DEFAULT 0 CHECK(scheduled_by_user IN (0,1));
UPDATE submissions SET scheduled_by_user=CASE WHEN json_valid(request) THEN json_type(request,'$.due_at') NOT IN ('null') AND json_type(request,'$.due_at') IS NOT NULL ELSE 0 END;
CREATE INDEX submissions_scheduled ON submissions(scheduled_by_user,due_at,id);
