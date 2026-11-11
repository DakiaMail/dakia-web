CREATE TABLE microsoft_subscriptions (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    subscription_id TEXT,
    client_state_hash TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    next_attempt INTEGER NOT NULL DEFAULT 0,
    wake_version INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX microsoft_subscription_id ON microsoft_subscriptions(subscription_id);
