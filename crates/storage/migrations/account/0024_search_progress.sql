-- No aggregate quota: previously blocked documents become eligible again.
UPDATE search_documents SET body_state='pending',retry_at=0 WHERE body_state='quota';
-- Maintain coverage without scanning all extracted bodies on each search.
ALTER TABLE search_totals ADD COLUMN headers INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_totals ADD COLUMN bodies INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_totals ADD COLUMN pending INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_totals ADD COLUMN gaps INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_totals ADD COLUMN retrying INTEGER NOT NULL DEFAULT 0;
ALTER TABLE search_totals ADD COLUMN dirty INTEGER NOT NULL DEFAULT 0;
UPDATE search_totals SET
 headers=(SELECT count(*) FROM search_documents),
 bodies=(SELECT count(*) FROM search_documents WHERE body_state='ready'),
 pending=(SELECT count(*) FROM search_documents WHERE body_state='pending'),
 gaps=(SELECT count(*) FROM search_documents WHERE body_state NOT IN ('pending','ready')),
 retrying=(SELECT count(*) FROM search_documents WHERE body_state='fetch_error'),
 dirty=(SELECT count(*) FROM search_dirty);
CREATE TRIGGER search_counts_insert AFTER INSERT ON search_documents BEGIN
 UPDATE search_totals SET headers=headers+1,bodies=bodies+(new.body_state='ready'),pending=pending+(new.body_state='pending'),gaps=gaps+(new.body_state NOT IN ('pending','ready')),retrying=retrying+(new.body_state='fetch_error') WHERE id=1;
END;
CREATE TRIGGER search_counts_delete AFTER DELETE ON search_documents BEGIN
 UPDATE search_totals SET headers=headers-1,bodies=bodies-(old.body_state='ready'),pending=pending-(old.body_state='pending'),gaps=gaps-(old.body_state NOT IN ('pending','ready')),retrying=retrying-(old.body_state='fetch_error') WHERE id=1;
END;
CREATE TRIGGER search_counts_update AFTER UPDATE OF body_state ON search_documents BEGIN
 UPDATE search_totals SET bodies=bodies+(new.body_state='ready')-(old.body_state='ready'),pending=pending+(new.body_state='pending')-(old.body_state='pending'),gaps=gaps+(new.body_state NOT IN ('pending','ready'))-(old.body_state NOT IN ('pending','ready')),retrying=retrying+(new.body_state='fetch_error')-(old.body_state='fetch_error') WHERE id=1;
END;
CREATE TRIGGER search_dirty_count_insert AFTER INSERT ON search_dirty BEGIN
 UPDATE search_totals SET dirty=dirty+1 WHERE id=1;
END;
CREATE TRIGGER search_dirty_count_delete AFTER DELETE ON search_dirty BEGIN
 UPDATE search_totals SET dirty=dirty-1 WHERE id=1;
END;
CREATE TABLE search_excluded_folders (name TEXT PRIMARY KEY);
INSERT INTO search_excluded_folders VALUES('Trash'),('Spam'),('Junk'),('[Gmail]/Trash'),('[Gmail]/Spam');
-- Reconsider existing content under the new eligibility policy.
INSERT OR IGNORE INTO search_dirty SELECT id FROM messages;
CREATE TRIGGER search_exclude_message_update AFTER UPDATE OF document ON messages
WHEN EXISTS(SELECT 1 FROM json_each(new.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)) BEGIN
 DELETE FROM search_documents WHERE message_id=new.id;
END;
DELETE FROM search_documents WHERE message_id IN (
 SELECT m.id FROM messages m,json_each(m.document,'$.folder_ids') f JOIN search_excluded_folders e ON lower(f.value)=lower(e.name)
);
