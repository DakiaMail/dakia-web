import contextlib
import fcntl
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native_upgrade', Path(__file__).parents[1] / 'upgrade-native.py')
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)


class NativeUpgrade(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.addCleanup(patch.stopall)
        for name, relative in [('APP', 'app'), ('DATA', 'data'), ('SECRETS', 'secrets'), ('CONFIG', 'config'), ('UNIT', 'systemd/dakia.service'), ('STATE', 'state'), ('BACKUPS', 'backups')]:
            patch.object(updater, name, self.root / relative).start()
        self.old = updater.APP / 'releases/v1.0.0'
        self.old.mkdir(parents=True)
        (updater.APP / 'current').symlink_to(self.old)
        self.files = ['dakia', 'dakia-server', 'dakia-admin', 'dakia-healthcheck', 'node', 'dakia.service']
        for name in self.files:
            (self.old / name).write_text('old-runtime')
        (self.old / 'VERSION').write_text('v1.0.0')
        for p in [updater.DATA, updater.SECRETS, updater.CONFIG, updater.UNIT.parent]:
            p.mkdir(parents=True, exist_ok=True)
        (updater.SECRETS / 'credential-key').write_bytes(b'k' * 32)
        (updater.CONFIG / 'dakia.env').write_text('DAKIA_ORIGIN=https://mail.test\n')
        updater.UNIT.write_text('old-runtime')
        self.live = {'DAKIA_DATA_DIR': str(updater.DATA), 'DAKIA_CREDENTIAL_KEY_FILE': str(updater.SECRETS / 'credential-key'), 'DAKIA_ORIGIN': 'https://mail.test'}
        self.mode = 'success'
        self.events = []
        self.payload = self.archive()
        self.metadata = {'tag_name': 'v1.1.0', 'draft': False, 'prerelease': False, 'assets': [{'name': n, 'state': 'uploaded', 'size': 1} for n in ['dakia-v1.1.0-linux-amd64.tar.gz', 'SHA256SUMS']]}
        for target, replacement in [('request', self.request), ('run', self.run_command), ('preflight', lambda old: (self.live, {'DAKIA_CREDENTIAL_KEY_FILE': updater.SECRETS / 'credential-key'})), ('live_environment', lambda release: self.live)]:
            patch.object(updater, target, replacement).start()
        patch.object(updater.os, 'geteuid', return_value=0).start()
        patch.object(updater.platform, 'machine', return_value='x86_64').start()
        patch.object(updater.shutil, 'chown').start()
        patch.object(updater.time, 'sleep').start()

    def archive(self, unsafe=False):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode='w:gz') as archive:
            directory = tarfile.TarInfo('web/assets')
            directory.type = tarfile.DIRTYPE
            directory.mode = 0o755
            archive.addfile(directory)
            for name in self.files + ['VERSION', 'REVISION']:
                body = ('v1.1.0' if name == 'VERSION' else 'new-runtime').encode()
                info = tarfile.TarInfo('../escape' if unsafe else name)
                info.size = len(body)
                info.mode = 0o755
                archive.addfile(info, io.BytesIO(body))
        return data.getvalue()

    def request(self, url):
        if self.mode == 'lookup-failure':
            raise OSError('network failed')
        if url.endswith('/latest'):
            return io.BytesIO(json.dumps(self.metadata).encode())
        if url.endswith('/SHA256SUMS'):
            digest = '0' * 64 if self.mode == 'checksum-failure' else hashlib.sha256(self.payload).hexdigest()
            return io.BytesIO(f'{digest}  dakia-v1.1.0-linux-amd64.tar.gz\n'.encode())
        return io.BytesIO(self.payload)

    def run_command(self, args, env=None):
        args = [str(a) for a in args]
        self.events.append((args, (updater.APP / 'current').resolve()))
        if args[0] == 'runuser':
            if self.mode == 'backup-failure':
                raise subprocess.CalledProcessError(1, args)
            snapshot = Path(args[-2])
            snapshot.mkdir()
            (snapshot / 'manifest.json').write_text('{}')
            (snapshot / 'control.sqlite').write_text('snapshot')
        if args == ['systemctl', 'start', 'dakia'] and self.mode == 'startup-failure' and (updater.APP / 'current').resolve() != self.old:
            (updater.DATA / 'migration-marker').write_text('new schema')
            raise subprocess.CalledProcessError(1, args)

    def upgrade(self):
        with contextlib.redirect_stdout(io.StringIO()):
            updater.upgrade()

    def test_backup_precedes_activation_and_preserves_secrets_and_configuration(self):
        self.upgrade()
        self.assertEqual((updater.APP / 'current').resolve().name, 'v1.1.0')
        self.assertEqual((updater.APP / 'current/web/assets').stat().st_mode & 0o777, 0o755)
        backups = list(updater.BACKUPS.iterdir())
        self.assertEqual(len(backups), 1)
        backup = backups[0]
        self.assertEqual(updater.BACKUPS.stat().st_mode & 0o777, 0o711)
        self.assertTrue((backup / 'COMPLETE').exists())
        self.assertEqual((backup / 'progress').read_text(), 'complete\n')
        self.assertEqual((backup / 'secrets/credential-key').read_bytes(), b'k' * 32)
        self.assertEqual((backup / 'config/dakia.env').read_bytes(), (updater.CONFIG / 'dakia.env').read_bytes())
        self.assertEqual((backup / 'dakia.service').read_text(), 'old-runtime')
        self.assertEqual(backup.stat().st_mode & 0o777, 0o700)
        self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in backup.rglob('*') if p.is_file()))
        backup_event = next(e for e in self.events if e[0][0] == 'runuser')
        self.assertEqual(backup_event[1], self.old)
        self.assertTrue(self.old.exists())
        self.assertFalse((updater.STATE / 'active.json').exists())

    def test_current_or_older_release_is_a_noop(self):
        for tag in ['v1.0.0', 'v0.9.0']:
            self.metadata['tag_name'] = tag
            self.metadata['assets'][0]['name'] = f'dakia-{tag}-linux-amd64.tar.gz'
            self.upgrade()
        self.assertEqual(self.events, [])
        self.assertFalse(updater.BACKUPS.exists())

    def test_lookup_and_checksum_failure_never_stop_service(self):
        for mode in ['lookup-failure', 'checksum-failure']:
            self.mode = mode
            with self.assertRaises((OSError, RuntimeError)):
                self.upgrade()
            self.assertEqual(self.events, [])
            self.assertEqual((updater.APP / 'current').resolve(), self.old)

    def test_backup_failure_restarts_only_unchanged_runtime(self):
        self.mode = 'backup-failure'
        with self.assertRaises(subprocess.CalledProcessError):
            self.upgrade()
        self.assertEqual((updater.APP / 'current').resolve(), self.old)
        self.assertTrue(all(release == self.old for _, release in self.events))
        self.assertIn(['systemctl', 'start', 'dakia'], [e[0] for e in self.events])
        self.assertFalse((updater.STATE / 'active.json').exists())

    def test_failure_after_activation_stops_new_version_and_blocks_retry(self):
        self.mode = 'startup-failure'
        with self.assertRaises(subprocess.CalledProcessError):
            self.upgrade()
        self.assertEqual((updater.APP / 'current').resolve().name, 'v1.1.0')
        self.assertTrue((updater.DATA / 'migration-marker').exists())
        self.assertEqual(self.events[-1][0], ['systemctl', 'stop', 'dakia'])
        self.assertTrue(all(release != self.old for args, release in self.events if args == ['systemctl', 'start', 'dakia']))
        with self.assertRaisesRegex(RuntimeError, 'interrupted upgrade'):
            self.upgrade()

    def test_path_traversal_archive_fails_before_downtime(self):
        self.payload = self.archive(unsafe=True)
        with self.assertRaises(tarfile.FilterError):
            self.upgrade()
        self.assertEqual(self.events, [])

    def test_insufficient_backup_space_fails_before_downtime(self):
        with patch.object(updater.shutil, 'disk_usage', return_value=SimpleNamespace(free=0)):
            with self.assertRaisesRegex(RuntimeError, 'Insufficient backup space'):
                self.upgrade()
        self.assertEqual(self.events, [])

    def test_staging_cleanup_failure_keeps_recovery_marker(self):
        temporary_directory = tempfile.TemporaryDirectory
        class FailedCleanup:
            def __init__(self, **kwargs):
                self.directory = temporary_directory(**kwargs)
            def __enter__(self):
                return self.directory.name
            def __exit__(self, *_args):
                self.directory.cleanup()
                raise OSError('staging cleanup failed')
        with patch.object(updater.tempfile, 'TemporaryDirectory', FailedCleanup):
            with self.assertRaisesRegex(OSError, 'staging cleanup failed'):
                self.upgrade()
        self.assertTrue((updater.STATE / 'active.json').exists())
        self.assertEqual(self.events[-1][0], ['systemctl', 'stop', 'dakia'])

    def test_concurrent_upgrader_is_refused(self):
        updater.STATE.mkdir()
        with (updater.STATE / 'lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(RuntimeError, 'Another native upgrade'):
                self.upgrade()
        self.assertEqual(self.events, [])

    def test_pending_configuration_and_external_databases_are_refused(self):
        self.live.update({'DAKIA_NODE': str(self.old / 'node'), 'DAKIA_WEB_ROOT': str(self.old / 'web'), 'DAKIA_WORKERS': str(self.old / 'workers'), 'DAKIA_SEARCH_MODEL_DIR': str(self.old / 'search-model'), 'DAKIA_MODEL_CACHE': str(updater.DATA / 'models'), 'DAKIA_LISTEN': '127.0.0.1:3025'})
        def output(args):
            if 'User' in args:
                return b'dakia\n'
            if 'DropInPaths' in args:
                return b''
            injected = {'DAKIA_NODE', 'DAKIA_WEB_ROOT', 'DAKIA_WORKERS', 'DAKIA_SEARCH_MODEL_DIR', 'DAKIA_MODEL_CACHE', 'DAKIA_LISTEN'}
            return b'\0'.join(f'{k}={v}'.encode() for k, v in self.live.items() if k not in injected) + b'\0'
        with patch.object(updater, 'output', output), patch.object(updater.platform, 'freedesktop_os_release', return_value={'ID': 'ubuntu', 'VERSION_ID': '24.04'}):
            # Exercise real preflight rather than the flow fixture's replacement.
            preflight = self.original_preflight
            preflight(self.old)
            self.live['DAKIA_NODE'] = str(updater.APP / 'current/node')
            preflight(self.old)
            with patch.object(updater, 'output', lambda args: b'DAKIA_ORIGIN=https://changed.test\0' if args[0] == 'systemd-run' else output(args)):
                with self.assertRaisesRegex(RuntimeError, 'pending'):
                    preflight(self.old)
            self.live['DAKIA_REMOVED_SETTING'] = 'old-value'
            with patch.object(updater, 'output', lambda args: output(args).replace(b'DAKIA_REMOVED_SETTING=old-value\0', b'')):
                with self.assertRaisesRegex(RuntimeError, 'pending'):
                    preflight(self.old)
            del self.live['DAKIA_REMOVED_SETTING']
            self.live['DAKIA_DATABASE_URL'] = 'postgres://fixture-only'
            with self.assertRaisesRegex(RuntimeError, 'External databases'):
                preflight(self.old)

    original_preflight = staticmethod(updater.preflight)


if __name__ == '__main__':
    unittest.main()
