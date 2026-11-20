-- Permanent connection-scoped idempotency bindings, not a delivery claim.
CREATE TABLE agent_send_requests (
 connection_id TEXT NOT NULL REFERENCES agent_connections(id) ON DELETE CASCADE,
 request_hash TEXT NOT NULL, payload_hash TEXT NOT NULL, account_id TEXT NOT NULL,
 operation_id TEXT NOT NULL UNIQUE,
 PRIMARY KEY(connection_id,request_hash)
);
