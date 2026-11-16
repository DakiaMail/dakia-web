#!/usr/bin/env python3
"""Fetch only the pinned SQLite CLI sources used by the repair image builder."""
import hashlib
from pathlib import Path
import sys
import urllib.request
import zipfile
import io

URL = 'https://www.sqlite.org/2026/sqlite-amalgamation-3540100.zip'
SHA256 = '0a9c8e34352dad46006aa82836d5383bff9dc6bc9ce5a60553c05aa30e6ad1f9'
FILES = ('sqlite3.c', 'sqlite3.h', 'shell.c')


def extract(data, destination):
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError('SQLite repair source checksum mismatch')
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for name in FILES:
            # Never extract arbitrary archive paths.
            (destination / name).write_bytes(archive.read('sqlite-amalgamation-3540100/' + name))


if __name__ == '__main__':
    with urllib.request.urlopen(URL, timeout=60) as response:
        data = response.read(32 * 1024 * 1024 + 1)
    extract(data, Path(sys.argv[1]))
