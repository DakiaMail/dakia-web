# Repair a damaged search index

Use this procedure only for the derived FTS search-index corruption described in the [SQLite engine note](../vendor/libsqlite3-sys/DAKIA.md). Updating the SQLite engine prevents the known failure but doesn't repair an already-damaged index. Other database corruption needs separate investigation.

Stop Dakia and make a complete backup, including its credential key, with the installed version's matching operator. Preserve the original data and test recovery in an isolated copy first. Never run a repair alongside the server or against a live Docker volume.

The offline helper requires Python 3 and a SQLite CLI version 3.54.1 or newer:

```sh
python3 scripts/repair-search-index.py \
  --data-dir /absolute/path/to/stopped-data \
  --backup-dir /absolute/path/to/new-private-repair-backup \
  --sqlite-cli /absolute/path/to/sqlite3
```

The backup directory must not exist. The helper takes the operator lock, checks the expected schema, snapshots each account database, rebuilds only derived postings, and checks integrity and unchanged non-FTS data. Unsupported integrity failures stop the repair. It doesn't contact providers or alter credentials, messages, or submission state.

If a check fails, keep the service stopped and inspect the private backup and error before proceeding. Don't automatically restore over newer data. After a successful repair, restart the matching application version and check search. See [backup and restore](../deploy/README.md#backup-and-restore).
