"""Synthetic offline repair tests. Set SQLITE_REPAIR_CLI to SQLite 3.54.1+."""
import importlib.util
import fcntl
import json
import sys
from contextlib import closing
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
TOOLS = Path(os.environ.get('DAKIA_TEST_REPAIR_TOOLS_DIR', ROOT / 'scripts'))
spec = importlib.util.spec_from_file_location('repair', TOOLS / 'repair-search-index.py')
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
        schema = ('CREATE TABLE messages(id TEXT PRIMARY KEY,document TEXT); '
                  'CREATE TABLE provider_documents(key TEXT,document TEXT); '
                  'CREATE TABLE authoritative(value INTEGER CHECK(value>0)); '
                  'INSERT INTO authoritative VALUES(7);')
        schema += (ROOT / 'crates/storage/migrations/account/0019_full_text_search.sql').read_text()
        schema += ''.join(f"INSERT INTO messages VALUES('{i}','{{}}'); INSERT INTO search_documents(message_id,source,sender,recipients,subject,body) VALUES('{i}','{i}','sender','','subject','synthetic café рус body');" for i in range(100))
        repair.sql(CLI, self.db, schema)

    def corrupt(self):
        # Deliberately alter raw shadow bytes for the corruption fixture. The
        # shipped CLI enables defensive mode and correctly refuses this write.
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

    def test_current_attachment_index_repairs_without_changing_extracted_text(self):
        migration = (ROOT / 'crates/storage/migrations/account/0025_search_content.sql').read_text()
        migration += (ROOT / 'crates/storage/migrations/account/0026_attachment_trigrams.sql').read_text()
        repair.sql(CLI, self.db, migration + "UPDATE search_documents SET participants='carbon@example.invalid',attachment_text='attachment confidential figures';")
        self.corrupt()
        before = repair.fingerprint(self.db)
        repair.repair(self.db, self.backup, CLI)
        self.assertEqual(repair.fingerprint(self.db), before)
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM search_fts WHERE search_fts MATCH 'confidential';"), '100')

    def test_unknown_schema_is_rejected(self):
        repair.sql(CLI, self.db, 'DROP TABLE search_fts')
        with self.assertRaises(ValueError):
            repair.repair(self.db, self.backup, CLI)
        self.assertFalse(self.backup.exists())

    def test_non_fts_integrity_failure_is_rejected_before_backup(self):
        repair.sql(CLI, self.db, 'PRAGMA ignore_check_constraints=ON; INSERT INTO authoritative VALUES(-1);')
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


class DestinationRepairTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.db = Path(self.tmp.name) / 'search.sqlite'
        self.backup = Path(self.tmp.name) / 'snapshot.sqlite'
        migrations = sorted((ROOT / 'crates/storage/migrations/search/sqlite').glob('*.sql'))
        repair.sql(CLI, self.db, ''.join(path.read_text() for path in migrations))
        repair.sql(CLI, self.db, "INSERT INTO documents(account_id,message_id,version,source,metadata,sender,recipients,subject,body,participants,attachment_text,received_at,state) VALUES('a','m',1,'source','{\"id\":\"m\",\"folder_ids\":[\"INBOX\"],\"unread\":true,\"starred\":false}','sender@example.invalid','carbon@example.invalid','visible subject','retained body','reply@example.invalid','retained attachment','2026','ready'); INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text,native_text,phrase_safe) SELECT account_id,message_id,content_version,5,3,0,'wneedle','f3n776e6565646c65',1 FROM documents; INSERT INTO chunks(account_id,message_id,source,model_revision,ordinal,text,embedding,content_version) SELECT account_id,message_id,source,'fixture',0,'retained passage',zeroblob(1536),content_version FROM documents;")

    def test_both_destination_indexes_rebuild_without_text_vector_or_progress_changes(self):
        indexes = ('lexical_terms', 'metadata_terms')
        before = repair.fingerprint(self.db, indexes)
        repair.sql(CLI, self.db, 'DELETE FROM lexical_terms WHERE rowid=1; DELETE FROM metadata_terms WHERE rowid=1;')
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH 'f3n776e6565646c65';"), '0')
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM metadata_terms WHERE metadata_terms MATCH 'subject:vis';"), '0')
        result = repair.repair(self.db, self.backup, CLI, destination=True)
        self.assertTrue(result['raw_backup_verified'])
        self.assertEqual(repair.fingerprint(self.db, indexes), before)
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH 'f3n776e6565646c65';"), '1')
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM metadata_terms WHERE metadata_terms MATCH 'subject:vis';"), '1')
        self.assertEqual(repair.sql(CLI, self.backup, "SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH 'f3n776e6565646c65';"), '0')
        self.assertEqual(repair.sql(CLI, self.db, 'SELECT length(embedding) FROM chunks;'), '1536')

    def test_previous_destination_layout_is_still_repairable(self):
        previous = Path(self.tmp.name) / 'previous.sqlite'
        migrations = sorted((ROOT / 'crates/storage/migrations/search/sqlite').glob('*.sql'))
        repair.sql(CLI, previous, ''.join(path.read_text() for path in migrations if int(path.name.split('_')[0]) < 18))
        repair.sql(CLI, previous, "INSERT INTO documents(account_id,message_id,version,source,metadata,sender,recipients,subject,body,participants,attachment_text,received_at,state) VALUES('a','old',1,'source','{}','','','','','','','2026','ready'); INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text) SELECT account_id,message_id,content_version,3,3,0,'wlegacy' FROM documents; DELETE FROM lexical_terms WHERE rowid=1;")
        repair.repair(previous,self.backup,CLI,destination=True)
        self.assertEqual(repair.sql(CLI, previous, "SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH 'wlegacy';"),'1')

    def test_repair_during_lexical_upgrade_keeps_old_chunks_out_of_new_index(self):
        repair.sql(CLI, self.db, "INSERT INTO lexical_chunks(account_id,message_id,content_version,revision,field,ordinal,text) VALUES('a','old',1,3,3,0,'wlegacy');")
        before = repair.fingerprint(self.db, ('lexical_terms', 'metadata_terms'))
        repair.repair(self.db, self.backup, CLI, destination=True)
        self.assertEqual(repair.fingerprint(self.db, ('lexical_terms', 'metadata_terms')), before)
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM lexical_terms;"), '1')
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH 'n776c6567616379';"), '0')

    def test_failed_second_index_check_rolls_back_both_rebuilds(self):
        repair.sql(CLI, self.db, 'DELETE FROM lexical_terms WHERE rowid=1; DELETE FROM metadata_terms WHERE rowid=1;')
        original = repair.sql
        def fail_check(cli, path, query):
            if query.startswith('BEGIN IMMEDIATE'):
                query = query.replace('SELECT result FROM repair_integrity;', "INSERT INTO repair_integrity VALUES('fault'); SELECT result FROM repair_integrity;")
            return original(cli, path, query)
        with patch.object(repair, 'sql', fail_check), self.assertRaises(subprocess.CalledProcessError):
            repair.repair(self.db, self.backup, CLI, destination=True)
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH 'f3n776e6565646c65';"), '0')
        self.assertEqual(repair.sql(CLI, self.db, "SELECT count(*) FROM metadata_terms WHERE metadata_terms MATCH 'subject:vis';"), '0')
        self.assertTrue(self.backup.exists())


    def test_cli_repairs_separate_search_file_without_sqlite_application_store(self):
        data = Path(self.tmp.name) / 'postgres-app-root'
        data.mkdir()
        output = subprocess.run([sys.executable, str(TOOLS / 'repair-search-index.py'), '--data-dir', str(data), '--backup-dir', str(data / 'backup'), '--sqlite-cli', str(CLI), '--search-sqlite', str(self.db)], check=True, capture_output=True, text=True)
        self.assertTrue(json.loads(output.stdout)['search']['non_fts_tables_unchanged'])
        self.assertTrue((data / 'backup/search.sqlite').is_file())
        self.assertFalse((data / 'control.sqlite').exists())

    def test_cli_refuses_live_application_lock_before_creating_backup(self):
        data = Path(self.tmp.name) / 'app-root'
        data.mkdir()
        before = repair.fingerprint(self.db, ('lexical_terms', 'metadata_terms'))
        with (data / 'operator.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            output = subprocess.run([sys.executable, str(TOOLS / 'repair-search-index.py'), '--data-dir', str(data), '--backup-dir', str(data / 'backup'), '--sqlite-cli', str(CLI), '--search-sqlite', str(self.db)], capture_output=True, text=True)
        self.assertNotEqual(output.returncode, 0)
        self.assertFalse((data / 'backup').exists())
        self.assertEqual(repair.fingerprint(self.db, ('lexical_terms', 'metadata_terms')), before)

    def test_unknown_destination_schema_is_refused_before_backup(self):
        repair.sql(CLI, self.db, "DROP TABLE metadata_terms; CREATE VIRTUAL TABLE metadata_terms USING fts5(subject);")
        with self.assertRaisesRegex(ValueError, 'Unexpected independent search schema'):
            repair.repair(self.db, self.backup, CLI, destination=True)
        self.assertFalse(self.backup.exists())


if __name__ == '__main__':
    unittest.main()
