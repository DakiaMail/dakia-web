CREATE TABLE search_cursors (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    document TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX search_cursor_expiry ON search_cursors(expires_at);
