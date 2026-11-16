-- Serialize setup and invitation claims on both supported database engines.
CREATE TABLE control_locks(name TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0);
INSERT INTO control_locks(name) VALUES('auth');
