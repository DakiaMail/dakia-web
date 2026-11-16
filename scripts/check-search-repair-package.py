#!/usr/bin/env python3
"""Exercise the copied repair CLI and its clients using disposable synthetic data."""
import argparse
import hashlib
from importlib.metadata import version
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tools-dir', type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument('--sqlite-cli', default='sqlite3')
    args = parser.parse_args()
    for tool in ('repair-search-index.py', 'repair_search_postgres.py'):
        if not (args.tools_dir / tool).is_file():
            raise ValueError('Packaged repair tool missing')
    if version('psycopg') != '3.3.8' or version('psycopg-binary') != '3.3.8':
        raise ValueError('Unexpected PostgreSQL adapter version')
    for client in ('pg_dump', 'pg_restore'):
        text = subprocess.check_output([client, '--version'], text=True)
        if int(text.split()[-1].split('.')[0]) < 17:
            raise ValueError('PostgreSQL 17 or newer clients required')
    sqlite = lambda db, text: subprocess.check_output([args.sqlite_cli, '-batch', '-bail', str(db), text], text=True).strip()
    if sqlite(':memory:', 'SELECT sqlite_version()') != '3.54.1':
        raise ValueError('Unexpected SQLite version')
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        data = root / 'data'
        data.mkdir()
        source = data / 'control.sqlite'
        source.write_bytes(b'synthetic application data remains unopened')
        source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
        for native_revision in (None, 4, 5):
            native = native_revision is not None
            name = f'native-{native_revision}' if native else 'legacy'
            search = root / f'{name}-search.sqlite'
            snapshot = root / f'{name}-snapshot'
            term = ('f3' if native_revision == 5 else '') + 'n776e6565646c65' if native else 'needle'
            revision_filter = " IN (4,5)" if native_revision == 5 else f"={native_revision}"
            lexical = f"CREATE TABLE lexical_chunks(id INTEGER PRIMARY KEY,text TEXT,native_text TEXT,revision INTEGER); INSERT INTO lexical_chunks VALUES(1,'wneedle','{term}',{native_revision}),(2,'wlegacy','n776c6567616379',{native_revision-1}); CREATE VIEW lexical_native_content AS SELECT id,native_text FROM lexical_chunks WHERE revision{revision_filter} AND native_text<>''; CREATE VIRTUAL TABLE lexical_terms USING fts5(native_text,content=lexical_native_content,content_rowid=id,tokenize='unicode61 remove_diacritics 0');" if native else "CREATE TABLE lexical_chunks(id INTEGER PRIMARY KEY,text TEXT); INSERT INTO lexical_chunks VALUES(1,'needle'); CREATE VIRTUAL TABLE lexical_terms USING fts5(text,content=lexical_chunks,content_rowid=id,tokenize='unicode61 remove_diacritics 0');"

            sqlite(search, "CREATE TABLE documents(rowid INTEGER PRIMARY KEY,sender TEXT,recipients TEXT,subject TEXT,body TEXT); CREATE TABLE chunks(id INTEGER PRIMARY KEY,embedding BLOB); CREATE TABLE progress(id INTEGER PRIMARY KEY,processed INTEGER); CREATE TABLE index_identity(id INTEGER PRIMARY KEY,value TEXT); INSERT INTO index_identity VALUES(1,'stable'); INSERT INTO documents VALUES(1,'sender','recipient','visible subject','retained text'); INSERT INTO chunks VALUES(1,zeroblob(1536)); INSERT INTO progress VALUES(1,1); CREATE VIRTUAL TABLE metadata_terms USING fts5(sender,recipients,subject,content=documents,content_rowid=rowid,tokenize='trigram case_sensitive 1');"+lexical)
            # Both postings are missing. Repair must use retained canonical fields.
            output = subprocess.check_output([sys.executable, str(args.tools_dir / 'repair-search-index.py'), '--data-dir', str(data), '--search-sqlite', str(search), '--backup-dir', str(snapshot), '--sqlite-cli', args.sqlite_cli], text=True)
            result = json.loads(output)['search']
            if not result['non_fts_tables_unchanged'] or not result['raw_backup_verified']:
                raise ValueError('Packaged repair did not preserve canonical state')
            expected = sqlite(search, f"SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH '{term}'; SELECT count(*) FROM metadata_terms WHERE metadata_terms MATCH 'subject:vis'; SELECT length(embedding) FROM chunks; SELECT processed FROM progress; SELECT value FROM index_identity;")
            if expected != '1\n1\n1536\n1\nstable' or hashlib.sha256(source.read_bytes()).hexdigest() != source_hash:
                raise ValueError('Packaged repair result mismatch')
            if sqlite(snapshot / 'search.sqlite', f"SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH '{term}';") != '0':
                raise ValueError('Packaged snapshot did not retain original postings')
            if native:
                legacy_count = sqlite(search,"SELECT count(*) FROM lexical_terms WHERE lexical_terms MATCH 'n776c6567616379';")
                if legacy_count != ('1' if native_revision == 5 else '0'):
                    raise ValueError('Packaged repair changed legacy projection availability')
            if native and sqlite(search,"SELECT count(*) FROM lexical_terms;") != ('2' if native_revision == 5 else '1'):
                raise ValueError('Packaged repair included legacy chunks during upgrade')
    print('Copied repair CLI, pinned clients, snapshot and canonical preservation passed')


if __name__ == '__main__':
    main()
