"""Disposable local comparison. Commands: setup, pl, dakia, stop.

Docker access and prebuilt cargo test binary required. Only fixture-labelled
containers are removed. All ports bind loopback. No owner mail is used.
"""
import argparse, pathlib, subprocess, sys

HERE = pathlib.Path(__file__).resolve().parent
IMAGE = 'ghcr.io/karatektus/pl_mail@sha256:fb0d453584d8e86d34459538b281bbc0aecab947c0d38864d662c549fcd0c9b9'
LABEL = 'dakia.initial-sync-benchmark=1'
p = argparse.ArgumentParser()
p.add_argument('action', choices=['setup', 'pl', 'dakia', 'stop'])
p.add_argument('--results', type=pathlib.Path, required=True)
p.add_argument('--binary', type=pathlib.Path)
p.add_argument('--count', type=int, default=50000)
p.add_argument('--port', type=int, default=1993)
p.add_argument('--delay-ms', type=float, default=0)
p.add_argument('--resources', action='store_true')
p.add_argument('--resource-interval', type=float, default=.25)
a = p.parse_args()
a.results = a.results.resolve()
a.results.mkdir(parents=True, exist_ok=True)

def run(*args, **kwargs):
    return subprocess.run(list(map(str, args)), check=True, **kwargs)
def docker(*args, **kwargs): return run('docker', *args, **kwargs)
def remove(name):
    found = subprocess.run(['docker','inspect','--format','{{index .Config.Labels "dakia.initial-sync-benchmark"}}',name],capture_output=True,text=True)
    if found.returncode == 0:
        if found.stdout.strip() != '1': raise RuntimeError('Refusing to remove non-fixture container '+name)
        docker('rm','-f','-v',name)
def start_imap(client):
    remove('dakia-initial-sync-imap')
    docker('run','-d','--name','dakia-initial-sync-imap','--label',LABEL,'--network','dakia-initial-sync','--network-alias','fake-imap','-p',f'127.0.0.1:{a.port}:1993','-v',f'{HERE}:/bench:ro','-v',f'{a.results}:/results','python:3.13-slim','python','/bench/fake_imap.py','--count',a.count,'--delay-ms',a.delay_ms,'--cert','/results/cert.pem','--key','/results/key.pem','--output',f'/results/{client}-imap.json')
    # Readiness checks neither open IMAP connections nor affect request totals.
    import time
    for _ in range(100):
        logs = subprocess.check_output(['docker','logs','dakia-initial-sync-imap'],text=True)
        if 'ready port=' in logs: return
        time.sleep(.1)
    raise RuntimeError('IMAP fixture failed readiness')

if a.action == 'setup':
    for name in ['dakia-initial-sync-pl','dakia-initial-sync-imap','dakia-initial-sync-db']: remove(name)
    docker('network','inspect','dakia-initial-sync',stdout=subprocess.DEVNULL) if subprocess.run(['docker','network','inspect','dakia-initial-sync'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode == 0 else docker('network','create','--label',LABEL,'dakia-initial-sync')
    run('openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',a.results/'key.pem','-out',a.results/'cert.pem','-days','2','-subj','/CN=localhost','-addext','subjectAltName=DNS:localhost,DNS:fake-imap','-addext','extendedKeyUsage=serverAuth','-addext','keyUsage=critical,digitalSignature,keyEncipherment','-addext','basicConstraints=critical,CA:FALSE',stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    docker('run','-d','--name','dakia-initial-sync-db','--label',LABEL,'--network','dakia-initial-sync','-e','POSTGRES_PASSWORD=fixture','-e','POSTGRES_USER=fixture','-e','POSTGRES_DB=fixture','postgres:18')
    docker('run','-d','--name','dakia-initial-sync-pl','--label',LABEL,'--network','dakia-initial-sync','-v',f'{HERE}:/bench:ro','-e','APP_ENV=prod','-e','APP_DEBUG=0','-e','APP_SECRET=synthetic-benchmark-only','-e','APP_ENCRYPTION_KEY=emL6R8zs0pSXCVV2HvVvbFubriWJIla7kgnqSGqCJ8E=','-e','DATABASE_URL=postgresql://fixture:fixture@dakia-initial-sync-db:5432/fixture?serverVersion=18&charset=utf8','-e','MESSENGER_TRANSPORT_DSN=doctrine://default?auto_setup=0','-e','MAILER_DSN=null://null','--entrypoint','sleep',IMAGE,'infinity')
    import time
    for _ in range(100):
        if subprocess.run(['docker','exec','dakia-initial-sync-db','pg_isready','-h','127.0.0.1','-U','fixture'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode == 0: break
        time.sleep(.2)
    else: raise RuntimeError('Database failed readiness')
    docker('exec','dakia-initial-sync-pl','php','bin/console','app:db:migrate','--no-interaction')
elif a.action == 'pl':
    start_imap('pl')
    docker('exec','dakia-initial-sync-pl','php','/bench/pl_mail.php','setup')
    monitor = None
    if a.resources:
        from resource_usage import DockerMonitor
        monitor = DockerMonitor(['dakia-initial-sync-pl', 'dakia-initial-sync-db'], a.results/'pl-resources.json', a.resource_interval)
        monitor.start()
    status = -1
    try:
        with open(a.results/'pl.log','w') as f:
            result = docker('exec','-e',f'BENCH_COUNT={a.count}','dakia-initial-sync-pl','php','/bench/pl_mail.php',stdout=f,stderr=subprocess.STDOUT)
            status = result.returncode
    finally:
        if monitor:
            monitor.finish(status)
elif a.action == 'dakia':
    if not a.binary: p.error('--binary required for Dakia; compile before timing')
    start_imap('dakia')
    extra = ['--resources', a.results/'dakia-resources.json', '--resource-interval', a.resource_interval] if a.resources else []
    run(sys.executable,HERE/'run_dakia.py','--binary',a.binary.resolve(),'--pl-result',a.results/'pl.log','--output',a.results/'dakia.log','--cert',a.results/'cert.pem','--count',a.count,'--port',a.port, *extra)
else:
    for name in ['dakia-initial-sync-pl','dakia-initial-sync-imap','dakia-initial-sync-db']: remove(name)
