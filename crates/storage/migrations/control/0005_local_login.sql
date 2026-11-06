CREATE TABLE local_login (
 id INTEGER PRIMARY KEY CHECK(id=1),
 user_id TEXT NOT NULL UNIQUE REFERENCES users(id),
 username TEXT NOT NULL UNIQUE,
 password_hash TEXT NOT NULL
);
CREATE TABLE login_throttle (
 id INTEGER PRIMARY KEY CHECK(id=1),
 window_start INTEGER NOT NULL,
 attempts INTEGER NOT NULL
);
