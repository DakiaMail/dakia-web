CREATE TABLE agent_connections (
 id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE,
 allow_send INTEGER NOT NULL DEFAULT 0 CHECK(allow_send IN (0,1)),
 allow_delete INTEGER NOT NULL DEFAULT 0 CHECK(allow_delete IN (0,1)),
 revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)), created_at BIGINT NOT NULL
);
CREATE INDEX agent_connection_owner ON agent_connections(user_id);
CREATE TABLE agent_audit (
 id TEXT PRIMARY KEY, at BIGINT NOT NULL, user_id TEXT NOT NULL,
 connection_id TEXT NOT NULL, action TEXT NOT NULL, outcome TEXT NOT NULL,
 account_id TEXT, resource_id TEXT, request_id TEXT
);
CREATE INDEX agent_audit_retention ON agent_audit(at);
