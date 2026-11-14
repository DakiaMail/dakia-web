"""Offline PostgreSQL search index repair. Requires psycopg[binary]==3.3.8.

Retains a native search-schema archive and canonical fingerprints. No provider,
extraction or inference calls. Never uses REINDEX CONCURRENTLY or repairs catalogs.
"""
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import subprocess

LEASE = 19217639535198209
SCHEMA = 'dakia_search'


def fingerprint(db):
    from psycopg import sql
    # Canonical reads must not trust an index suspected of being corrupt.
    db.execute('SET LOCAL enable_indexscan=off; SET LOCAL enable_indexonlyscan=off; SET LOCAL enable_bitmapscan=off')
    tables = db.execute("SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=%s AND c.relkind='r' ORDER BY c.relname COLLATE \"C\"", (SCHEMA,)).fetchall()
    result = {}
    for (table,) in tables:
        keys = db.execute("SELECT a.attname FROM pg_index i JOIN pg_class c ON c.oid=i.indrelid JOIN pg_namespace n ON n.oid=c.relnamespace CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY k(attnum,ordinal) JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum=k.attnum WHERE n.nspname=%s AND c.relname=%s AND i.indisprimary ORDER BY k.ordinal", (SCHEMA, table)).fetchall()
        if not keys:
            raise ValueError('Search table without a primary key')
        query = sql.SQL('COPY (SELECT * FROM {}.{} ORDER BY {}) TO STDOUT (FORMAT BINARY)').format(sql.Identifier(SCHEMA), sql.Identifier(table), sql.SQL(',').join(sql.Identifier(key[0]) for key in keys))
        digest = hashlib.sha256()
        with db.cursor().copy(query) as stream:
            for block in stream:
                digest.update(block)
        result[table] = digest.hexdigest()
    for name, in db.execute("SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=%s AND c.relkind='S' ORDER BY c.relname COLLATE \"C\"", (SCHEMA,)).fetchall():
        result['sequence:' + name] = db.execute(sql.SQL('SELECT last_value,is_called FROM {}.{}').format(sql.Identifier(SCHEMA), sql.Identifier(name))).fetchone()
    return result


def native_tool(arguments, url):
    # Translate explicitly supplied libpq parameters to their native environment
    # names. PGDATABASE alone does not expand a URI for pg_dump. Credentials
    # remain outside argv, tracebacks and command output.
    from psycopg.pq import Conninfo
    env = dict(os.environ)
    for option in Conninfo.parse(url.encode()):
        if option.val is not None:
            if not option.envvar:
                raise ValueError('Connection option has no native client environment mapping')
            env[option.envvar.decode()] = option.val.decode()
    output = subprocess.run([str(v) for v in arguments], env=env, capture_output=True)
    if output.returncode:
        raise RuntimeError('PostgreSQL archive command failed')
    return output.stdout


def database_identity(db):
    address, port, name = db.execute("SELECT inet_server_addr()::text,current_setting('port'),current_database()").fetchone()
    return (address or db.info.host, port, name)


def repair(url, backup, pg_dump, pg_restore, application_url=None):
    import psycopg
    from psycopg import sql
    backup = Path(backup)
    if backup.exists():
        raise FileExistsError(backup)
    with ExitStack() as stack:
        leases = set()
        # Exclude online application instances, including other data roots and
        # hosts, whenever the application uses PostgreSQL. Search table locks
        # also exclude writers from a separately configured application store.
        for target in dict.fromkeys(filter(None, [application_url, url])):
            db = stack.enter_context(psycopg.connect(target, autocommit=True))
            identity = database_identity(db)
            if identity not in leases:
                if not db.execute('SELECT pg_try_advisory_lock(%s)', (LEASE,)).fetchone()[0]:
                    raise RuntimeError('Database is in use by an application or operator')
                leases.add(identity)
        with db.transaction():
            db.execute("SET LOCAL lock_timeout='5s'; SET LOCAL search_path=pg_catalog,dakia_search,public")
            tables = db.execute("SELECT c.relname,c.relkind FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=%s AND c.relkind IN ('r','p','f','m') ORDER BY c.relname COLLATE \"C\"", (SCHEMA,)).fetchall()
            if not tables or any(kind != 'r' for _, kind in tables):
                raise ValueError('Unexpected search schema')
            db.execute(sql.SQL('LOCK TABLE {} IN SHARE MODE NOWAIT').format(sql.SQL(',').join(sql.Identifier(SCHEMA, name) for name, _ in tables)))
            required = {'documents', 'chunks', 'lexical_chunks', 'index_identity', 'embedding_jobs', 'lexical_jobs', 'embedding_receipts', 'document_folders'}
            if not required.issubset({name for name, _ in tables}):
                raise ValueError('Incomplete search schema')
            if not db.execute('SELECT value FROM dakia_search.index_identity WHERE id=1').fetchone():
                raise ValueError('Missing search identity')
            indexes = db.execute("SELECT ci.relname,am.amname FROM pg_index i JOIN pg_class ci ON ci.oid=i.indexrelid JOIN pg_namespace n ON n.oid=ci.relnamespace JOIN pg_am am ON am.oid=ci.relam WHERE n.nspname=%s ORDER BY ci.relname COLLATE \"C\"", (SCHEMA,)).fetchall()
            methods = {name: method for name, method in indexes}
            if any(method not in ('btree', 'gin', 'hnsw') for _, method in indexes) or not {'gin', 'hnsw'}.issubset(set(methods.values())):
                raise ValueError('Unexpected search index methods')
            if methods.get('lexical_terms') != 'gin' or methods.get('chunks_ann') != 'hnsw':
                raise ValueError('Missing native search indexes')
            before = fingerprint(db)
            snapshot = db.execute('SELECT pg_export_snapshot()').fetchone()[0]
            backup.touch(mode=0o600, exist_ok=False)
            native_tool([pg_dump, '--format=custom', '--schema=dakia_search', '--no-owner', '--no-privileges', '--snapshot=' + snapshot, '--file=' + str(backup)], url)
            toc = native_tool([pg_restore, '--list', backup], url).decode()
            if any(f'TABLE DATA dakia_search {table} ' not in toc for table, _ in tables):
                raise ValueError('Incomplete search archive')
            digest = hashlib.sha256()
            with backup.open('rb') as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b''):
                    digest.update(block)
                os.fsync(stream.fileno())
            manifest = {'format': 1, 'archive_sha256': digest.hexdigest(), 'canonical': before, 'indexes': methods}
            manifest_path = backup.with_suffix(backup.suffix + '.json')
            with open(manifest_path, 'x', opener=lambda p, flags: os.open(p, flags, 0o600)) as stream:
                json.dump(manifest, stream, sort_keys=True)
                stream.flush()
                os.fsync(stream.fileno())
            directory = os.open(backup.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
            # Every non-concurrent single-index rebuild belongs to this one
            # transaction. A later failure rolls back all earlier replacements.
            for name, _ in indexes:
                db.execute(sql.SQL('REINDEX INDEX {}').format(sql.Identifier(SCHEMA, name)))
            if before != fingerprint(db):
                raise ValueError('Canonical search state changed during repair')
            valid = db.execute("SELECT bool_and(i.indisvalid AND i.indisready AND i.indislive) FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=%s", (SCHEMA,)).fetchone()[0]
            if not valid:
                raise ValueError('Native search indexes remain invalid')
        return {'canonical_tables_unchanged': True, 'archive_catalog_verified': True, 'archive_sha256': digest.hexdigest(), 'indexes_rebuilt': len(indexes), 'atomic': True}
