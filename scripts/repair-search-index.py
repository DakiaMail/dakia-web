#!/usr/bin/env python3
"""Offline derived-postings repair. No provider access or owner-state rewrites.

Stop the app first. Uses its operator lock and requires SQLite 3.54.1+ CLI.
Keeps a raw SQLite snapshot before each repair, including the damaged postings.
"""
import argparse
from contextlib import closing
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess


def connect(path):
    return sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True)


def fingerprint(path):
    """All non-FTS tables, including search_documents, plus the full schema."""
    with closing(connect(path)) as db:
        schema = db.execute('SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name').fetchall()
        result: dict[str, object] = {'schema': hashlib.sha256(json.dumps(schema).encode()).hexdigest()}
        for (name,) in db.execute("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT IN ('search_fts','search_fts_data','search_fts_idx','search_fts_docsize','search_fts_config') ORDER BY name"):
            quoted = '"' + name.replace('"', '""') + '"'
            columns = len(db.execute(f'SELECT * FROM {quoted} LIMIT 0').description)
            order = ','.join(str(i + 1) for i in range(columns))
            h = hashlib.sha256()
            count = 0
            for row in db.execute(f'SELECT * FROM {quoted} ORDER BY {order}'):
                values = [('blob', v.hex()) if isinstance(v, bytes) else (type(v).__name__, v) for v in row]
                h.update(json.dumps(values, ensure_ascii=True, separators=(',', ':')).encode() + b'\n')
                count += 1
            result[name] = (count, h.hexdigest())
        result['foreign_key_check'] = db.execute('PRAGMA foreign_key_check').fetchall()
        return result


def sql(cli, path, query):
    return subprocess.run([str(cli), '-batch', '-bail', str(path), query],
                          check=True, capture_output=True, text=True).stdout.strip()


def repair(path, backup, cli):
    version = tuple(map(int, sql(cli, ':memory:', 'SELECT sqlite_version();').split('.')))
    if version < (3, 54, 1):
        raise ValueError('Use SQLite 3.54.1 or newer, not the corrupting application engine')
    with closing(connect(path)) as db:
        definition = db.execute("SELECT sql FROM sqlite_schema WHERE name='search_fts'").fetchone()
        expected = "CREATE VIRTUAL TABLE search_fts USING fts5(sender,recipients,subject,body,content=search_documents,content_rowid=rowid,tokenize='trigram case_sensitive 1')"
        if not definition or definition[0] != expected:
            raise ValueError('Unexpected search schema; refuse generic database repair')
    before = fingerprint(path)
    integrity = sql(cli, path, 'PRAGMA integrity_check;').splitlines()
    # Never reinterpret ordinary database corruption as a repairable index error.
    for line in integrity:
        if line != 'ok' and line != 'malformed inverted index for FTS5 table main.search_fts' and not (
                line.startswith('fts5: corruption found reading blob ') and line.endswith(' from table "search_fts"')):
            raise ValueError('Integrity failure outside the supported derived FTS index')
    if backup.exists():
        raise FileExistsError(backup)
    # A database snapshot includes committed WAL pages. No credentials are printed.
    backup.touch(mode=0o600, exist_ok=False)
    with closing(connect(path)) as source, closing(sqlite3.connect(backup)) as destination:
        source.backup(destination)
    if fingerprint(backup) != before:
        raise ValueError('Backup logical verification failed')
    # Rebuild and both checks are in ONE transaction. A failed check/CLI exits
    # without COMMIT, leaving the live index unchanged. No canonical SQL runs.
    output = sql(cli, path, "BEGIN IMMEDIATE; INSERT INTO search_fts(search_fts) VALUES('rebuild'); "
                 "INSERT INTO search_fts(search_fts,rank) VALUES('integrity-check',1); "
                 "CREATE TEMP TABLE repair_integrity(result TEXT CHECK(result='ok')); "
                 "INSERT INTO repair_integrity SELECT * FROM pragma_integrity_check; "
                 "SELECT result FROM repair_integrity; COMMIT;")
    if output != 'ok':
        raise ValueError('Post-rebuild database integrity failure')
    if fingerprint(path) != before:
        raise ValueError('Non-FTS data changed; stop and reconcile, never restore automatically')
    if sql(cli, path, 'PRAGMA integrity_check;') != 'ok':
        raise ValueError('Committed database integrity failure')
    return {'integrity': 'ok', 'non_fts_tables_unchanged': True, 'raw_backup_verified': True,
            'sqlite': '.'.join(map(str, version)), 'pre_repair_integrity': integrity}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-dir', required=True, type=Path)
    parser.add_argument('--backup-dir', required=True, type=Path)
    parser.add_argument('--sqlite-cli', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    # Same lock as the server and dakia-admin. Never operate concurrently.
    with (args.data_dir / 'operator.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        args.backup_dir.mkdir(mode=0o700)
        with closing(connect(args.data_dir / 'control.sqlite')) as db:
            ids = [r[0] for r in db.execute('SELECT id FROM accounts ORDER BY id')]
        results = []
        for i, account in enumerate(ids):
            import uuid
            if str(uuid.UUID(account)) != account:
                raise ValueError('Invalid account database identifier')
            result = repair(args.data_dir / 'accounts' / f'{account}.sqlite',
                            args.backup_dir / f'account-{i}.sqlite', args.sqlite_cli)
            results.append({'account_index': i, **result})
        print(json.dumps({'accounts': results, 'owner_mail_mutations': 0}, indent=2))


if __name__ == '__main__':
    main()
