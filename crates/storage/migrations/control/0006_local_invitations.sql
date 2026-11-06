-- Preserve the original owner, password hash, user ID, mailboxes and sessions.
CREATE TABLE local_login_next (
 id INTEGER PRIMARY KEY,
 user_id TEXT NOT NULL UNIQUE REFERENCES users(id),
 username TEXT NOT NULL UNIQUE,
 password_hash TEXT NOT NULL,
 is_owner INTEGER NOT NULL DEFAULT 0 CHECK(is_owner IN (0,1))
);
INSERT INTO local_login_next(id,user_id,username,password_hash,is_owner)
 SELECT id,user_id,username,password_hash,1 FROM local_login;
DROP TABLE local_login;
ALTER TABLE local_login_next RENAME TO local_login;
CREATE TABLE local_invitations (
 id TEXT PRIMARY KEY,
 token_hash TEXT NOT NULL UNIQUE,
 created_by TEXT NOT NULL REFERENCES users(id),
 expires_at INTEGER NOT NULL
);
CREATE TABLE auth_attempts (
 bucket TEXT PRIMARY KEY,
 window_start INTEGER NOT NULL,
 attempts INTEGER NOT NULL
);
