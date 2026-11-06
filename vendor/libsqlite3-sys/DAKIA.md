# Dakia SQLite engine pin

Rust wrapper: crates.io libsqlite3-sys 0.30.1, MIT, preserved LICENSE and source.
Only sqlite3/sqlite3.c, sqlite3.h and sqlite3ext.h are replaced with unmodified
SQLite 3.54.1, public domain, from:
https://www.sqlite.org/2026/sqlite-amalgamation-3540100.zip

Official sqlite3.c SHA3-256:
e2b869d3003b33c942b7dd7063604fbc53553e1b66609570df39b9b7b558ee81
Release: https://www.sqlite.org/releaselog/3_54_1.html

SQLx 0.8.6 depends on the 0.30 wrapper, whose original engine is SQLite 3.46.0.
Normal external-content FTS5 secure-delete update/delete churn reproduces actual
index corruption with that engine, independently readable as corrupt on 3.46.1
and 3.54.1. The same deterministic workloads pass with newer engines. Dakia keeps
secure deletion, trigram semantics, SQLx and its existing Rust API. The engine
pin also includes upstream fixes released since 3.46.1. Existing damaged derived
postings require a separately backed-up offline rebuild; upgrading alone does
not repair them. See docs/search-index-repair.md.

The checked-in bindings expose the older compatible C API, which Dakia uses.
Use sqlite3_libversion()/SQL sqlite_version() for the running engine version,
not the wrapper's generated header-version constants. New SQLite APIs are not
exposed by these bindings. Bundled SQLite is the supported configuration here;
unused SQLCipher sources and registry metadata are intentionally excluded.
Do not enable SQLCipher features on this local package. Remove this local patch
when SQLx supports an appropriately current engine directly, after running the
secure-delete churn regression and storage/server suites.
