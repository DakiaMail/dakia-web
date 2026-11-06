#!/usr/bin/env python3
"""End-to-end thread benchmark using only the disposable loopback Dovecot fixture.

Set DAKIA_FIXTURE_IMAPS_PORT and DAKIA_FIXTURE_CERT as documented in
crates/mail-provider/fixtures/README.md. Never accepts a remote hostname.
"""
import imaplib
import os
from pathlib import Path
import ssl
import subprocess
import uuid

root = Path(__file__).resolve().parents[3]
prefix = "DakiaGraph-" + uuid.uuid4().hex[:10]
context = ssl.create_default_context(cafile=os.environ["DAKIA_FIXTURE_CERT"])
with imaplib.IMAP4_SSL("localhost", int(os.environ["DAKIA_FIXTURE_IMAPS_PORT"]), ssl_context=context) as imap:
    imap.login("fixture", "fixture-only-password")
    for index in range(4):
        folder = f"{prefix}-{index}"
        assert imap.create(folder)[0] == "OK"
        for message in range(index, 10, 4):
            references = " ".join(f"<m{i}@{prefix}.test>" for i in range(message))
            raw = (f"From: fixture@example.test\r\nTo: friend@example.test\r\n"
                   f"Subject: Synthetic thread performance\r\nDate: 28 Oct 2026 10:00:00 +0000\r\n"
                   f"Message-ID: <m{message}@{prefix}.test>\r\nReferences: {references}\r\n\r\n"
                   "Disposable synthetic body, never needed for discovery.\r\n").encode()
            assert imap.append(folder, None, None, raw)[0] == "OK"
env = dict(os.environ, DAKIA_THREAD_FIXTURE_PREFIX=prefix)
subprocess.run(["cargo", "test", "-p", "dakia-server", "disposable_wire_thread_graph_performance", "--", "--ignored", "--nocapture"], cwd=root, env=env, check=True)
print("Synthetic-only folders retained:", prefix)
