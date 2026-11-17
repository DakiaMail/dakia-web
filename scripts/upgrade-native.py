#!/usr/bin/env python3
"""Upgrade a standard native installation, retaining an offline recovery backup."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import signal
import shutil
import subprocess
import tarfile
import tempfile
import time
import urllib.request

APP = Path('/opt/dakia')
DATA = Path('/var/lib/dakia/data')
SECRETS = Path('/var/lib/dakia-secrets')
CONFIG = Path('/etc/dakia')
UNIT = Path('/etc/systemd/system/dakia.service')
STATE = Path('/var/lib/dakia-upgrades')
BACKUPS = Path('/var/lib/dakia/backups')
REPOSITORY = 'https://github.com/DakiaMail/dakia-web/releases/download'


def output(args):
    return subprocess.check_output(args)


def run(args, env=None):
    subprocess.run([str(a) for a in args], env=env, check=True)


def version(tag):
    if not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag):
        raise RuntimeError('Automatic upgrades require stable vX.Y.Z native releases')
    return tuple(map(int, tag[1:].split('.')))


def request(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'Dakia native upgrade'}), timeout=30)


def latest(arch):
    with request('https://api.github.com/repos/DakiaMail/dakia-web/releases/latest') as response:
        release = json.loads(response.read(4 * 1024 * 1024))
    tag = release['tag_name']
    version(tag)
    assets = {a['name'] for a in release.get('assets', []) if a.get('state') == 'uploaded' and a.get('size', 0) > 0}
    if release.get('draft') or release.get('prerelease') or not {f'dakia-{tag}-linux-{arch}.tar.gz', 'SHA256SUMS'} <= assets:
        raise RuntimeError('Latest stable release is missing its native archive or checksums')
    return tag


def prepare(tag, arch, work):
    archive = f'dakia-{tag}-linux-{arch}.tar.gz'
    base = f'{REPOSITORY}/{tag}'
    with request(f'{base}/SHA256SUMS') as response:
        lines = response.read(65536).decode().splitlines()
    matches = [line.split() for line in lines if len(line.split()) == 2 and line.split()[1] == archive]
    if len(matches) != 1 or not re.fullmatch(r'[a-fA-F0-9]{64}', matches[0][0]):
        raise RuntimeError('Expected exactly one valid archive checksum')
    digest = hashlib.sha256()
    payload = work / archive
    with request(f'{base}/{archive}') as response, payload.open('wb') as target:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            target.write(chunk)
    if digest.hexdigest() != matches[0][0].lower():
        raise RuntimeError('Native archive checksum mismatch; service was not stopped')
    unpacked = work / 'runtime'
    unpacked.mkdir()
    with tarfile.open(payload) as bundle:
        bundle.extractall(unpacked, filter='data')
    # The safe tar filter leaves directories subject to our private backup umask.
    for path in [unpacked, *unpacked.rglob('*')]:
        if path.is_dir() and not path.is_symlink():
            path.chmod(0o755)
    for name in ['VERSION', 'REVISION', 'dakia', 'dakia-server', 'dakia-admin', 'dakia-healthcheck', 'node', 'dakia.service']:
        if not (unpacked / name).is_file():
            raise RuntimeError(f'Native release is missing {name}')
    if (unpacked / 'VERSION').read_text().strip() != tag:
        raise RuntimeError('Archive version does not match the selected release')
    return unpacked


def live_environment(release):
    pid = int(output(['systemctl', 'show', 'dakia', '-p', 'MainPID', '--value']).strip())
    if not pid or Path(f'/proc/{pid}/exe').resolve() != release / 'dakia-server':
        raise RuntimeError('Start the installed Dakia service before upgrading')
    return dict(item.decode().split('=', 1) for item in Path(f'/proc/{pid}/environ').read_bytes().split(b'\0') if b'=' in item)


def preflight(old):
    os_release = platform.freedesktop_os_release()
    if os_release.get('ID') != 'ubuntu' or os_release.get('VERSION_ID') != '24.04':
        raise RuntimeError('Native upgrades require Ubuntu 24.04')
    if output(['systemctl', 'show', 'dakia', '-p', 'User', '--value']).strip() != b'dakia':
        raise RuntimeError('Custom service users require a manual upgrade')
    if output(['systemctl', 'show', 'dakia', '-p', 'DropInPaths', '--value']).strip() or UNIT.read_bytes() != (old / 'dakia.service').read_bytes():
        raise RuntimeError('Custom systemd units require a manual upgrade')
    live = live_environment(old)
    configured = output(['systemd-run', '--quiet', '--wait', '--pipe', '--property=EnvironmentFile=' + str(CONFIG / 'dakia.env'), '/usr/bin/env', '-0'])
    configured = dict(item.decode().split('=', 1) for item in configured.split(b'\0') if item.startswith(b'DAKIA_') and b'=' in item)
    runtime_paths = {'DAKIA_NODE', 'DAKIA_WORKERS', 'DAKIA_WEB_ROOT', 'DAKIA_SEARCH_MODEL_DIR'}
    if runtime_paths & configured.keys():
        raise RuntimeError('Custom runtime paths require a manual upgrade')
    expected = dict(configured)
    for key, value in {'DAKIA_NODE': str(old / 'node'), 'DAKIA_WEB_ROOT': str(old / 'web'), 'DAKIA_WORKERS': str(old / 'workers'), 'DAKIA_SEARCH_MODEL_DIR': str(old / 'search-model'), 'DAKIA_MODEL_CACHE': str(DATA / 'models'), 'DAKIA_LISTEN': '127.0.0.1:3025'}.items():
        expected.setdefault(key, value)
    actual = {k: v for k, v in live.items() if k.startswith('DAKIA_')}
    for key in runtime_paths:
        if key in actual:
            actual[key] = str(Path(actual[key]).resolve())
        expected[key] = str(Path(expected[key]).resolve())
    if actual != expected:
        raise RuntimeError('Apply pending Dakia configuration changes before upgrading')
    if Path(live['DAKIA_DATA_DIR']).resolve() != DATA.resolve() or Path(live['DAKIA_CREDENTIAL_KEY_FILE']).resolve() != (SECRETS / 'credential-key').resolve():
        raise RuntimeError('Custom data or key locations require a manual upgrade')
    if live.get('DAKIA_DATABASE_URL') or live.get('DAKIA_SEARCH_DATABASE_URL'):
        raise RuntimeError('External databases require a manual upgrade')
    files = {key: Path(value) for key, value in live.items() if key.startswith('DAKIA_') and key.endswith('_FILE') and value}
    if any(not p.is_absolute() or not p.is_file() for p in files.values()):
        raise RuntimeError('Configured secret files must be readable regular files')
    return live, files


def wait_healthy(release, env):
    env = dict(env, LD_LIBRARY_PATH=str(release))
    for _ in range(30):
        try:
            run([release / 'dakia-healthcheck'], env=env)
            live_environment(release)
            return
        except (subprocess.CalledProcessError, RuntimeError, FileNotFoundError):
            time.sleep(1)
    raise RuntimeError('Dakia did not become healthy within 30 seconds')


def backup(old, live, files, destination):
    destination.mkdir(mode=0o700)
    shutil.chown(destination, user='dakia', group='dakia')
    env = dict(os.environ, **{k: v for k, v in live.items() if k.startswith('DAKIA_')})
    env['LD_LIBRARY_PATH'] = str(old)
    run(['runuser', '-u', 'dakia', '--', old / 'dakia-admin', 'backup', DATA, destination / 'snapshot', SECRETS / 'credential-key'], env=env)
    if not (destination / 'snapshot/manifest.json').is_file():
        raise RuntimeError('Offline backup did not produce a complete manifest')
    shutil.copytree(SECRETS, destination / 'secrets')
    shutil.copytree(CONFIG, destination / 'config')
    shutil.copy2(UNIT, destination / 'dakia.service')
    (destination / 'runtime.json').write_text(json.dumps({'release': str(old), 'version': (old / 'VERSION').read_text().strip(), 'environment': {k: v for k, v in live.items() if k.startswith('DAKIA_')}}))
    extras = destination / 'extra-files'
    extras.mkdir(mode=0o700)
    mapping = {}
    for index, (name, path) in enumerate(files.items()):
        shutil.copy2(path, extras / str(index))
        mapping[name] = {'path': str(path), 'backup': str(index)}
    (extras / 'paths.json').write_text(json.dumps(mapping))
    for path in [destination, *destination.rglob('*')]:
        if path.is_symlink():
            raise RuntimeError('Backup unexpectedly contains a symlink')
        shutil.chown(path, user='root', group='root')
        path.chmod(0o700 if path.is_dir() else 0o600)
    (destination / 'COMPLETE').write_text('Offline data, secrets, configuration and unit backup completed\n')


def upgrade():
    if os.geteuid() != 0:
        raise RuntimeError('Run the native upgrader as root inside the Dakia LXC')
    os.umask(0o077)
    if STATE.is_symlink():
        raise RuntimeError('Upgrade state must not be a symlink')
    STATE.mkdir(mode=0o700, exist_ok=True)
    shutil.chown(STATE, user='root', group='root')
    STATE.chmod(0o700)
    lock = os.open(STATE / 'lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError('Another native upgrade is running') from error
        active = STATE / 'active.json'
        if active.exists():
            raise RuntimeError(f'An interrupted upgrade needs recovery; inspect {active}')
        current = APP / 'current'
        old = current.resolve(strict=True)
        if not current.is_symlink() or old.parent != APP / 'releases':
            raise RuntimeError('Expected a versioned native installation under /opt/dakia/releases')
        installed = (old / 'VERSION').read_text().strip()
        version(installed)
        arch = {'x86_64': 'amd64', 'aarch64': 'arm64'}.get(platform.machine())
        if not arch:
            raise RuntimeError('Unsupported architecture')
        tag = latest(arch)
        if version(tag) <= version(installed):
            print(f'Dakia {installed} is current; no service restart or backup needed')
            return
        live, files = preflight(old)
        target = APP / 'releases' / tag
        if target.exists():
            raise RuntimeError(f'Release directory already exists: {target}; inspect it before retrying')
        if BACKUPS.is_symlink():
            raise RuntimeError('Backup directory must not be a symlink')
        BACKUPS.mkdir(mode=0o755, parents=True, exist_ok=True)
        shutil.chown(BACKUPS, user='root', group='root')
        # The service user must traverse this parent to write its private snapshot.
        BACKUPS.chmod(0o711)
        if BACKUPS.resolve().is_relative_to(DATA.resolve()):
            raise RuntimeError('Backup directory overlaps the live data directory')
        key_before = (SECRETS / 'credential-key').read_bytes()
        destination = BACKUPS / f'{time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())}-{time.time_ns()}'
        stopped = activated = False
        try:
            with tempfile.TemporaryDirectory(prefix='.upgrade-', dir=APP / 'releases') as temporary:
                work = Path(temporary)
                staged = prepare(tag, arch, work)
                sources = [DATA / 'control.sqlite', DATA / 'accounts', DATA / 'durable', SECRETS, CONFIG, *files.values()]
                required = sum(p.stat().st_size for source in sources for p in ([source] if source.is_file() else source.rglob('*')) if p.is_file())
                if shutil.disk_usage(BACKUPS).free < required * 2 + 64 * 1024 * 1024:
                    raise RuntimeError('Insufficient backup space; retain room for a separate restore')
                state = {'from': str(old), 'to': str(target), 'backup': str(destination), 'phase': 'backup'}
                active.write_text(json.dumps(state))
                stopped = True
                run(['systemctl', 'stop', 'dakia'])
                backup(old, live, files, destination)
                (destination / 'progress').write_text('backup-complete\n')
                os.rename(staged, target)
                for path in [target, *target.rglob('*')]:
                    if not path.is_symlink():
                        path.chmod(path.stat().st_mode & ~0o022)
                target.chmod(0o755)
                state['phase'] = 'activating'
                active.write_text(json.dumps(state))
                # Mark activation first: an interruption must never trigger an unsafe downgrade.
                activated = True
                link = APP / '.current-upgrade'
                link.symlink_to(target)
                os.replace(link, current)
                shutil.copyfile(target / 'dakia.service', UNIT)
                UNIT.chmod(0o644)
                run(['systemctl', 'daemon-reload'])
                run(['systemctl', 'start', 'dakia'])
                wait_healthy(target, live)
                if (SECRETS / 'credential-key').read_bytes() != key_before:
                    raise RuntimeError('Credential key changed during upgrade')
                (destination / 'progress').write_text('complete\n')
        except BaseException:
            if activated:
                try:
                    run(['systemctl', 'stop', 'dakia'])
                except Exception:
                    print('Could not stop Dakia; stop the failed service before recovery')
                print(f'Upgrade failed after activation. Do not start the old runtime on migrated data. Recovery backup: {destination}')
            elif stopped:
                run(['systemctl', 'start', 'dakia'])
                wait_healthy(old, live)
                active.unlink(missing_ok=True)
                print('Upgrade failed before activation; the unchanged version was restarted')
            raise
        active.unlink()
        print(f'Upgraded {installed} to {tag}. Private recovery backup: {destination}')
    finally:
        os.close(lock)


if __name__ == '__main__':
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt('Native upgrade interrupted')
    signal.signal(signal.SIGTERM, interrupted)
    try:
        upgrade()
    except Exception as error:
        raise SystemExit(str(error)) from error
