CREATE TABLE push_endpoints (id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id), document TEXT NOT NULL, UNIQUE(user_id,id));
