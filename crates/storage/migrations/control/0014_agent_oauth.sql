CREATE TABLE agent_oauth_clients (id TEXT PRIMARY KEY, document TEXT NOT NULL, expires_at BIGINT NOT NULL);
CREATE TABLE agent_oauth_codes (
 code_hash TEXT PRIMARY KEY, connection_id TEXT NOT NULL REFERENCES agent_connections(id) ON DELETE CASCADE,
 client_id TEXT NOT NULL, redirect_uri TEXT NOT NULL, challenge TEXT NOT NULL, resource TEXT NOT NULL, expires_at BIGINT NOT NULL
);
CREATE TABLE agent_oauth_tokens (
 access_hash TEXT PRIMARY KEY, refresh_hash TEXT NOT NULL UNIQUE,
 connection_id TEXT NOT NULL REFERENCES agent_connections(id) ON DELETE CASCADE,
 client_id TEXT NOT NULL, resource TEXT NOT NULL, expires_at BIGINT NOT NULL, refresh_expires_at BIGINT NOT NULL
);
