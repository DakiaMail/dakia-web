"""Synthetic offline repair tests. Set SQLITE_REPAIR_CLI to SQLite 3.54.1+."""
import importlib.util
from contextlib import closing
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('repair', ROOT / 'scripts/repair-search-index.py')
assert spec and spec.loader
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)
CLI = Path(os.environ['SQLITE_REPAIR_CLI']).resolve()


class RepairTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.db = Path(self.tmp.name) / 'account.sqlite'
        self.backup = Path(self.tmp.name) / 'backup.sqlite'
        with closing(sqlite3.connect(self.db)) as db, db:
            db.executescript('CREATE TABLE messages(id TEXT PRIMARY KEY,document TEXT); '
                             'CREATE TABLE provider_documents(key TEXT,document TEXT); '
                             'CREATE TABLE authoritative(value INTEGER CHECK(value>0)); '
                             'INSERT INTO authoritative VALUES(7);')
            db.executescript((ROOT / 'crates/storage/migrations/account/0019_full_text_search.sql').read_text())
            for i in range(100):
                db.execute("INSERT INTO messages VALUES(?,'{}')", (str(i),))
                db.execute("INSERT INTO search_documents(message_id,source,sender,recipients,subject,body) VALUES(?,?,'sender','','subject','synthetic café рус body')", (str(i), str(i)))

    def corrupt(self):
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("UPDATE search_fts_data SET block=x'00' WHERE id>10")

    def test_repairs_only_derived_postings_with_verified_raw_backup(self):
        self.corrupt()
        before = repair.fingerprint(self.db)
        result = repair.repair(self.db, self.backup, CLI)
        self.assertEqual(result['integrity'], 'ok')
        self.assertEqual(repair.fingerprint(self.db), before)
        self.assertEqual(repair.fingerprint(self.backup), before)
        self.assertNotEqual(repair.sql(CLI, self.backup, 'PRAGMA integrity_check;'), 'ok')
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM search_fts WHERE search_fts MATCH 'synthetic';"), '100')

    def test_healthy_and_repeated_repair_preserve_source(self):
        before = repair.fingerprint(self.db)
        repair.repair(self.db, self.backup, CLI)
        repair.repair(self.db, self.backup.with_suffix('.second'), CLI)
        self.assertEqual(repair.fingerprint(self.db), before)
        with self.assertRaises(FileExistsError):
            repair.repair(self.db, self.backup, CLI)

    def test_unknown_schema_is_rejected(self):
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute('DROP TABLE search_fts')
        with self.assertRaises(ValueError):
            repair.repair(self.db, self.backup, CLI)
        self.assertFalse(self.backup.exists())

    def test_non_fts_integrity_failure_is_rejected_before_backup(self):
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute('PRAGMA ignore_check_constraints=ON')
            db.execute('INSERT INTO authoritative VALUES(-1)')
        with self.assertRaises(ValueError):
            repair.repair(self.db, self.backup, CLI)
        self.assertFalse(self.backup.exists())

    def test_failed_transaction_does_not_commit_rebuild(self):
        self.corrupt()
        original = repair.sql
        before = self.db.read_bytes()
        def fail_check(cli, path, query):
            if query.startswith('BEGIN IMMEDIATE'):
                query = query.replace('SELECT result FROM repair_integrity;',
                                      "INSERT INTO repair_integrity VALUES('fault'); SELECT result FROM repair_integrity;")
            return original(cli, path, query)
        with patch.object(repair, 'sql', fail_check), self.assertRaises(subprocess.CalledProcessError):
            repair.repair(self.db, self.backup, CLI)
        self.assertEqual(self.db.read_bytes(), before)
        self.assertTrue(self.backup.exists())


if __name__ == '__main__':
    unittest.main()
