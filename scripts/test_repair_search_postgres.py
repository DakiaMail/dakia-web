"""Native isolated PostgreSQL repair, atomic rollback and archive restore contracts."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import urlsplit, urlunsplit
import psycopg

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import repair_search_postgres as repair
URL = os.environ['DAKIA_TEST_REPAIR_POSTGRES_URL']
assert urlsplit(URL).path == '/dakia_repair_test'
PG_DUMP = Path(os.environ.get('DAKIA_TEST_PG_DUMP', '/opt/homebrew/opt/libpq/bin/pg_dump'))
PG_RESTORE = Path(os.environ.get('DAKIA_TEST_PG_RESTORE', '/opt/homebrew/opt/libpq/bin/pg_restore'))

class NativeRepairTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.backup = Path(self.tmp.name) / 'search.dump'
        with psycopg.connect(URL, autocommit=True) as db:
            db.execute('DROP SCHEMA IF EXISTS dakia_search CASCADE; CREATE SCHEMA dakia_search; CREATE EXTENSION IF NOT EXISTS vector SCHEMA public; CREATE EXTENSION IF NOT EXISTS pg_trgm SCHEMA public; SET search_path=dakia_search,public')
            for migration in sorted((ROOT / 'crates/storage/migrations/search/postgres').glob('*.sql')):
                db.execute(migration.read_text())
            db.execute("INSERT INTO documents(account_id,message_id,version,source,metadata,sender,recipients,subject,body,participants,attachment_text,received_at,state) VALUES('a','m',1,'source','{\"id\":\"m\",\"folder_ids\":[\"INBOX\"],\"unread\":true,\"starred\":false}','sender@example.invalid','carbon@example.invalid','visible subject','retained body','reply@example.invalid','retained attachment','2026','ready')")
            db.execute("INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) SELECT account_id,message_id,content_version,5,3,0,'wneedle','f3n776e6565646c65',TRUE FROM documents")
            vector = '[' + ','.join(['1'] + ['0'] * 383) + ']'
            db.execute("INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version) SELECT account_id,message_id,source,'fixture',0,'retained passage',%s::vector,content_version FROM documents", (vector,))

    def cli(self, data, env):
        image = os.environ.get('DAKIA_TEST_REPAIR_CONTAINER')
        if image:
            converted = dict(env)
            container_url = os.environ['DAKIA_TEST_REPAIR_CONTAINER_URL']
            source = urlsplit(URL)
            target = urlsplit(container_url)
            for name in ('DAKIA_DATABASE_URL', 'DAKIA_SEARCH_DATABASE_URL'):
                value = converted.get(name, '')
                if value.startswith(('postgres:', 'postgresql:')):
                    converted[name] = value.replace(source.netloc.split('@')[-1], target.netloc.split('@')[-1])
            command = ['docker', 'run', '--rm', '--user', f'{os.getuid()}:{os.getgid()}', '--network', os.environ['DAKIA_TEST_REPAIR_NETWORK'], '-v', f'{self.tmp.name}:{self.tmp.name}', '--env', 'DAKIA_DATABASE_URL', '--env', 'DAKIA_SEARCH_DATABASE_URL', image, '--data-dir', str(data), '--backup-dir', str(data / 'backup'), '--search-postgres']
            return subprocess.run(command, env=converted, capture_output=True, text=True)
        return subprocess.run([sys.executable, str(ROOT / 'scripts/repair-search-index.py'), '--data-dir', str(data), '--backup-dir', str(data / 'backup'), '--search-postgres', '--pg-dump', str(PG_DUMP), '--pg-restore', str(PG_RESTORE)], env=env, capture_output=True, text=True)

    def files(self):
        with psycopg.connect(URL) as db:
            return db.execute("SELECT c.relname,c.relfilenode FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='dakia_search' AND c.relkind='i' ORDER BY c.relname").fetchall()

    def fingerprint(self, url=URL):
        with psycopg.connect(url) as db:
            db.execute('SET search_path=pg_catalog,dakia_search,public')
            return repair.fingerprint(db)

    def test_reindex_preserves_canonical_state_and_archive_restores_exactly(self):
        before = self.fingerprint()
        files = self.files()
        if os.environ.get('DAKIA_TEST_REPAIR_CONTAINER'):
            data = Path(self.tmp.name) / 'archive-data'
            data.mkdir()
            output = self.cli(data, dict(os.environ, DAKIA_SEARCH_DATABASE_URL=URL, DAKIA_DATABASE_URL=URL))
            self.assertEqual(output.returncode, 0, output.stderr)
            result = json.loads(output.stdout)['search']
            self.backup = data / 'backup/search.dump'
        else:
            result = repair.repair(URL, self.backup, PG_DUMP, PG_RESTORE)
        self.assertTrue(result['atomic'])
        self.assertEqual(self.fingerprint(), before)
        self.assertNotEqual(self.files(), files)
        with psycopg.connect(URL) as db:
            db.execute('SET search_path=dakia_search,public; SET enable_seqscan=off')
            self.assertEqual(db.execute("SELECT message_id FROM lexical_chunks WHERE lexemes @@ to_tsquery('simple','f3n776e6565646c65')").fetchall(), [('m',)])
            self.assertEqual(db.execute("SELECT message_id FROM chunks ORDER BY embedding <=> (SELECT embedding FROM chunks LIMIT 1) LIMIT 1").fetchall(), [('m',)])

        self.assertEqual(self.backup.stat().st_mode & 0o777, 0o600)
        manifest = json.loads(self.backup.with_suffix('.dump.json').read_text())
        self.assertEqual(manifest['archive_sha256'], result['archive_sha256'])
        parts = urlsplit(URL)
        restored = urlunsplit(parts._replace(path='/dakia_repair_restore_test'))
        with psycopg.connect(URL, autocommit=True) as db:
            db.execute('DROP DATABASE IF EXISTS dakia_repair_restore_test WITH (FORCE)')
            db.execute('CREATE DATABASE dakia_repair_restore_test')
        try:
            with psycopg.connect(restored, autocommit=True) as db:
                db.execute('CREATE EXTENSION vector; CREATE EXTENSION pg_trgm')
            repair.native_tool([PG_RESTORE, '--exit-on-error', '--dbname=' + restored, self.backup], restored)
            self.assertEqual(self.fingerprint(restored), before)
        finally:
            with psycopg.connect(URL, autocommit=True) as db:
                db.execute('DROP DATABASE dakia_repair_restore_test WITH (FORCE)')

    def test_failure_after_earlier_reindex_rolls_back_every_replacement(self):
        before = self.fingerprint()
        files = self.files()
        original = psycopg.Connection.execute
        replacements = []
        def injected(db, query, *args, **kwargs):
            text = query.as_string(db) if hasattr(query, 'as_string') else query
            if text.startswith('REINDEX INDEX'):
                if replacements:
                    raise RuntimeError('injected later index failure')
                replacements.append(text)
            return original(db, query, *args, **kwargs)
        with patch.object(psycopg.Connection, 'execute', injected), self.assertRaisesRegex(RuntimeError, 'injected'):
            repair.repair(URL, self.backup, PG_DUMP, PG_RESTORE)
        self.assertTrue(replacements)
        self.assertTrue(self.backup.exists())
        self.assertEqual(self.files(), files)
        self.assertEqual(self.fingerprint(), before)

    def test_online_application_lease_refuses_before_backup(self):
        with psycopg.connect(URL, autocommit=True) as db:
            self.assertTrue(db.execute('SELECT pg_try_advisory_lock_shared(%s)', (repair.LEASE,)).fetchone()[0])
            with self.assertRaisesRegex(RuntimeError, 'in use'):
                repair.repair(URL, self.backup, PG_DUMP, PG_RESTORE, URL)
        self.assertFalse(self.backup.exists())

    def test_writer_in_separate_application_database_refuses_before_backup(self):
        with psycopg.connect(URL) as writer:
            writer.execute("UPDATE dakia_search.documents SET subject=subject WHERE message_id='m'")
            with self.assertRaises(psycopg.errors.LockNotAvailable):
                repair.repair(URL, self.backup, PG_DUMP, PG_RESTORE)
            writer.rollback()
        self.assertFalse(self.backup.exists())

    def test_cli_repairs_postgres_search_without_a_sqlite_application_store(self):
        data = Path(self.tmp.name) / 'data'
        data.mkdir()
        env = dict(os.environ, DAKIA_SEARCH_DATABASE_URL=URL, DAKIA_DATABASE_URL=URL)
        output = self.cli(data, env)
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertTrue(json.loads(output.stdout)['search']['canonical_tables_unchanged'])
        self.assertFalse((data / 'control.sqlite').exists())

    def test_equivalent_application_connection_reuses_database_lease(self):
        alias = URL.replace('postgres://', 'postgresql://', 1)
        before = self.fingerprint()
        result = repair.repair(URL, self.backup, PG_DUMP, PG_RESTORE, alias)
        self.assertTrue(result['atomic'])
        self.assertEqual(self.fingerprint(), before)

    def test_cli_with_sqlite_application_preserves_application_file(self):
        data = Path(self.tmp.name) / 'sqlite-data'
        data.mkdir()
        sentinel = data / 'control.sqlite'
        sentinel.write_bytes(b'application is not opened by search repair')
        before = sentinel.read_bytes()
        env = dict(os.environ, DAKIA_SEARCH_DATABASE_URL=URL, DAKIA_DATABASE_URL='sqlite:control.sqlite')
        output = self.cli(data, env)
        self.assertEqual(output.returncode, 0, output.stderr)
        self.assertEqual(sentinel.read_bytes(), before)
        self.assertTrue(json.loads(output.stdout)['search']['archive_catalog_verified'])

    def test_cli_failure_does_not_print_connection_credentials(self):
        data = Path(self.tmp.name) / 'failure-data'
        data.mkdir()
        secret = 'fixture-password-must-not-appear'
        parts = urlsplit(URL)
        userinfo,host = parts.netloc.rsplit('@',1)
        broken = urlunsplit(parts._replace(netloc=userinfo.split(':',1)[0]+':'+secret+'@'+host))
        self.assertNotEqual(broken,URL)
        env = dict(os.environ, DAKIA_SEARCH_DATABASE_URL=broken, DAKIA_DATABASE_URL='')
        output = self.cli(data, env)
        self.assertNotEqual(output.returncode, 0)
        self.assertNotIn(secret, output.stdout + output.stderr)
        self.assertFalse((data / 'backup/search.dump').exists())


if __name__ == '__main__':
    unittest.main()
