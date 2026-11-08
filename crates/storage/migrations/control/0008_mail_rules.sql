CREATE TABLE mail_rules (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    preset TEXT NOT NULL,
    sender TEXT NOT NULL,
    document TEXT NOT NULL,
    UNIQUE(user_id, preset, sender)
);
ALTER TABLE accounts ADD COLUMN connected_at TEXT;
