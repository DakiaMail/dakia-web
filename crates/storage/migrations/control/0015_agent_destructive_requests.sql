-- A claimed destructive action is never replayed after interruption.
CREATE TABLE agent_destructive_requests (
 connection_id TEXT NOT NULL REFERENCES agent_connections(id) ON DELETE CASCADE,
 request_hash TEXT NOT NULL, payload_hash TEXT NOT NULL, result TEXT,
 PRIMARY KEY(connection_id,request_hash)
);
