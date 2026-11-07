-- Keep execution receipts when mail is restored or a rule is disabled.
CREATE TABLE rule_executions (
    rule_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    operation_id TEXT UNIQUE,
    state TEXT NOT NULL CHECK(state IN ('queued','conflict','cancelled')),
    PRIMARY KEY(rule_id, message_id)
);
CREATE TABLE rule_inbox_observations (message_id TEXT PRIMARY KEY);
