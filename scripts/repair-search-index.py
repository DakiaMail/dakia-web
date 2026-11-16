#!/usr/bin/env python3
"""Offline derived-postings repair. No provider access or owner-state rewrites.

Stop the app first. Uses its operator lock. SQLite requires a 3.54.1+ CLI;
PostgreSQL requires psycopg and compatible pg_dump/pg_restore clients.
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


def fingerprint(path, indexes=('search_fts',)):
    """Canonical tables, vectors and progress, plus the full schema."""
    with closing(connect(path)) as db:
        schema = db.execute('SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name').fetchall()
        result: dict[str, object] = {'schema': hashlib.sha256(json.dumps(schema).encode()).hexdigest()}
        shadows = {name + suffix for name in indexes for suffix in ('', '_data', '_idx', '_docsize', '_config')}
        for (name,) in db.execute("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name"):
            if name in shadows:
                continue
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


def repair(path, backup, cli, destination=False):
    indexes = ('lexical_terms', 'metadata_terms') if destination else ('search_fts',)
    version = tuple(map(int, sql(cli, ':memory:', 'SELECT sqlite_version();').split('.')))
    if version < (3, 54, 1):
        raise ValueError('Use SQLite 3.54.1 or newer, not the corrupting application engine')
    with closing(connect(path)) as db:
        if destination:
            expected = {
                'lexical_terms': "CREATE VIRTUAL TABLE lexical_terms USING fts5(text,content=lexical_chunks,content_rowid=id,tokenize='unicode61 remove_diacritics 0')",
                'metadata_terms': "CREATE VIRTUAL TABLE metadata_terms USING fts5(sender,recipients,subject,content=documents,content_rowid=rowid,tokenize='trigram case_sensitive 1')",
            }
            columns = {row[1] for row in db.execute('PRAGMA table_info(lexical_chunks)')}
            if 'native_text' in columns:
                expected['lexical_terms'] = "CREATE VIRTUAL TABLE lexical_terms USING fts5(native_text,content=lexical_native_content,content_rowid=id,tokenize='unicode61 remove_diacritics 0')"
                view = db.execute("SELECT sql FROM sqlite_schema WHERE type='view' AND name='lexical_native_content'").fetchone()
                valid_views = {f"CREATE VIEW lexical_native_content AS SELECT id,native_text FROM lexical_chunks WHERE revision={revision} AND native_text<>''" for revision in (4, 5)}
                valid_views.add("CREATE VIEW lexical_native_content AS SELECT id,native_text FROM lexical_chunks WHERE revision IN (4,5) AND native_text<>''")
                if not view or view[0] not in valid_views:
                    raise ValueError('Unexpected independent search schema')
            identity = db.execute("SELECT value FROM index_identity WHERE id=1").fetchone()
            if not identity or not identity[0]:
                raise ValueError('Missing search storage identity')
            for name, definition_sql in expected.items():
                definition = db.execute("SELECT sql FROM sqlite_schema WHERE name=?", (name,)).fetchone()
                if not definition or definition[0] != definition_sql:
                    raise ValueError('Unexpected independent search schema')
        else:
            definition = db.execute("SELECT sql FROM sqlite_schema WHERE name='search_fts'").fetchone()
            expected = "CREATE VIRTUAL TABLE search_fts USING fts5(sender,recipients,subject,body,participants,mime_recipients,attachment_text,content=search_documents,content_rowid=rowid,tokenize='trigram case_sensitive 1')"
            if not definition or definition[0] not in (expected, expected.replace("body,participants,mime_recipients,attachment_text,content=", "body,content=")):
                raise ValueError('Unexpected search schema; refuse generic database repair')
    before = fingerprint(path, indexes)
    integrity = sql(cli, path, 'PRAGMA integrity_check;').splitlines()
    # Never reinterpret ordinary database corruption as a repairable index error.
    for line in integrity:
        if line != 'ok' and line not in {f'malformed inverted index for FTS5 table main.{name}' for name in indexes} and not (
                line.startswith('fts5: corruption found reading blob ') and any(line.endswith(f' from table "{name}"') for name in indexes)):
            raise ValueError('Integrity failure outside the supported derived FTS index')
    if backup.exists():
        raise FileExistsError(backup)
    # A database snapshot includes committed WAL pages. No credentials are printed.
    backup.touch(mode=0o600, exist_ok=False)
    with closing(connect(path)) as source, closing(sqlite3.connect(backup)) as backup_db:
        source.backup(backup_db)
    if fingerprint(backup, indexes) != before:
        raise ValueError('Backup logical verification failed')
    # Rebuild and both checks are in ONE transaction. A failed check/CLI exits
    # without COMMIT, leaving the live index unchanged. No canonical SQL runs.
    rebuild = ''.join(f"INSERT INTO {name}({name}) VALUES('rebuild'); INSERT INTO {name}({name},rank) VALUES('integrity-check',1); " for name in indexes)
    output = sql(cli, path, "BEGIN IMMEDIATE; " + rebuild +
                 "CREATE TEMP TABLE repair_integrity(result TEXT CHECK(result='ok')); "
                 "INSERT INTO repair_integrity SELECT * FROM pragma_integrity_check; "
                 "SELECT result FROM repair_integrity; COMMIT;")
    if output != 'ok':
        raise ValueError('Post-rebuild database integrity failure')
    if fingerprint(path, indexes) != before:
        raise ValueError('Non-FTS data changed; stop and reconcile, never restore automatically')
    if sql(cli, path, 'PRAGMA integrity_check;') != 'ok':
        raise ValueError('Committed database integrity failure')
    return {'integrity': 'ok', 'non_fts_tables_unchanged': True, 'raw_backup_verified': True,
            'sqlite': '.'.join(map(str, version)), 'pre_repair_integrity': integrity}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-dir', required=True, type=Path)
    parser.add_argument('--backup-dir', required=True, type=Path)
    parser.add_argument('--sqlite-cli', type=Path)
    parser.add_argument('--search-sqlite', type=Path, help='Repair the selected independent SQLite search file instead of legacy account indexes')
    parser.add_argument('--search-postgres', action='store_true', help='Repair the selected PostgreSQL search schema using environment connection URLs')
    parser.add_argument('--pg-dump', default='pg_dump', type=Path)
    parser.add_argument('--pg-restore', default='pg_restore', type=Path)
    args = parser.parse_args()
    if args.search_postgres and args.search_sqlite:
        parser.error('choose one search backend')
    if not args.search_postgres and not args.sqlite_cli:
        parser.error('--sqlite-cli is required for SQLite repair')
    os.umask(0o077)
    # Same lock as the server and dakia-admin. Never operate concurrently.
    with (args.data_dir / 'operator.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        args.backup_dir.mkdir(mode=0o700)
        if args.search_postgres:
            from repair_search_postgres import repair as repair_postgres
            application = os.environ.get('DAKIA_DATABASE_URL') or None
            application = application if application and application.startswith(('postgres:', 'postgresql:')) else None
            search = os.environ.get('DAKIA_SEARCH_DATABASE_URL') or application
            if not search or not search.startswith(('postgres:', 'postgresql:')):
                parser.error('PostgreSQL search connection is required in the environment')
            try:
                result = repair_postgres(search, args.backup_dir / 'search.dump', args.pg_dump, args.pg_restore, application)
            except Exception as error:
                # Never echo a connection URI, native stderr or content values.
                raise SystemExit(f'PostgreSQL search repair failed ({type(error).__name__}); no automatic restore was attempted') from None
            print(json.dumps({'search': result, 'owner_mail_mutations': 0}, indent=2))
            return
        if args.search_sqlite:
            result = repair(args.search_sqlite, args.backup_dir / 'search.sqlite', args.sqlite_cli, destination=True)
            print(json.dumps({'search': result, 'owner_mail_mutations': 0}, indent=2))
            return
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
