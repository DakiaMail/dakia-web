CREATE TABLE microsoft_oauth_flows (
    state_hash TEXT PRIMARY KEY,
    session_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id),
    verifier TEXT NOT NULL,
    email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    account_id TEXT,
    expires_at BIGINT NOT NULL
);
CREATE TABLE microsoft_subscriptions (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    subscription_id TEXT,
    client_state_hash TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    next_attempt BIGINT NOT NULL DEFAULT 0,
    wake_version BIGINT NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX microsoft_subscription_id ON microsoft_subscriptions(subscription_id);
