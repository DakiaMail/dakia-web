CREATE TABLE recipient_evidence (
    source_id TEXT NOT NULL,
    email TEXT NOT NULL COLLATE NOCASE,
    document TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY(source_id,email)
);
CREATE INDEX recipients_email ON recipient_evidence(email);
