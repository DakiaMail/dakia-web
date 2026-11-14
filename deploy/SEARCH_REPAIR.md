# Search repair

Stop all instances using the selected application and search stores. Keep the
application data directory and selected search database in place. The tool keeps
a backup and never restores automatically. Choose a new, private backup directory.

Build the operator image from a source checkout or an extracted runtime bundle:

```sh
docker build -f repair.Dockerfile -t dakia-search-repair .
# From a source checkout, use -f deploy/repair.Dockerfile instead.
```

The default image includes PostgreSQL 17 clients, SQLite 3.54.1 and pinned Python
dependencies. PostgreSQL clients must support the target server version. Set the
`POSTGRES_IMAGE` build argument to a pinned compatible PostgreSQL image when needed.

Run as the user who owns the data. Mount the application data, selected search
file and backup parent directory. For default SQLite application/search storage:

```sh
docker run --rm --user "$(id -u):$(id -g)" \
  -v /absolute/data:/data -v /absolute/backups:/backups \
  dakia-search-repair --data-dir /data \
  --search-sqlite /data/search/index.sqlite \
  --backup-dir /backups/new-search-snapshot --sqlite-cli /usr/local/bin/sqlite3
```

For PostgreSQL search, pass the existing connection environment without putting
credentials in arguments. Configure a container network that reaches the database;
`localhost` inside the operator container refers to that container.

```sh
docker run --rm --user "$(id -u):$(id -g)" \
  --env DAKIA_DATABASE_URL --env DAKIA_SEARCH_DATABASE_URL \
  -v /absolute/data:/data -v /absolute/backups:/backups \
  dakia-search-repair --data-dir /data --search-postgres \
  --backup-dir /backups/new-search-snapshot
```

An unset PostgreSQL search connection follows the PostgreSQL application connection.
For mixed backends, select the search mode explicitly. The local application lock,
PostgreSQL application leases and destination table locks reject active writers.
Separate application roots and remote SQLite users also need to be stopped.

SQLite repair validates the schema, keeps a raw snapshot and rebuilds both FTS
indexes atomically. During a lexical upgrade it preserves searchable old-format
postings alongside the new format. PostgreSQL repair keeps a custom-format search-schema archive
and checksum manifest, then rebuilds native indexes in one transaction. Retained
text, embeddings, identities, jobs, receipts and progress must remain unchanged.
An archive restore needs pgvector and pg_trgm installed in the destination.

Read the JSON result and retain the backup, manifest and command transcript.
Failures leave evidence in place and do not trigger an automatic restore. Resolve
canonical corruption or incompatible schemas before retrying. Accelerator snapshots
are rebuilt from durable SQLite vectors when missing or corrupt; this command
repairs postings and native database indexes.
