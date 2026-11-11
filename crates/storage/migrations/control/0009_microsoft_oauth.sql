CREATE TABLE microsoft_oauth_flows (
    state_hash TEXT PRIMARY KEY,
    session_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id),
    verifier TEXT NOT NULL,
    email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    account_id TEXT,
    expires_at INTEGER NOT NULL
);
