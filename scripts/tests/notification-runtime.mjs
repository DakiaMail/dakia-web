// Disposable real-runtime acceptance. Never points at an existing installation.
// Usage: node scripts/tests/notification-runtime.mjs /absolute/native/runtime
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:net';
import { randomBytes } from 'node:crypto';
const runtime = resolve(process.argv[2]);
const directory = await mkdtemp(join(tmpdir(), 'dakia-real-notifications-'));
async function freePort() {
  const server = createServer();
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const port = server.address().port;
  await new Promise(r => server.close(r));
  return port;
}
const origin = 'https://mail.test';
async function instance(name, existingFile) {
  const dir = join(directory, name);
  await mkdir(dir, { mode: 0o700 });
  for (const sub of ['data', 'secrets']) await mkdir(join(dir, sub), { mode: 0o700 });
  const port = await freePort();
  const env = { ...process.env, DAKIA_DATA_DIR: join(dir, 'data'), DAKIA_CREDENTIAL_KEY_FILE: join(dir, 'secrets/credential-key'), DAKIA_ORIGIN: origin, DAKIA_LISTEN: `127.0.0.1:${port}`, DAKIA_RELEASE_CHECKS: 'false', ...(existingFile ? { DAKIA_VAPID_FILE: existingFile } : {}) };
  if (!existingFile) delete env.DAKIA_VAPID_FILE;
  delete env.DAKIA_VAPID_SUBJECT;
  const file = existingFile || join(dir, 'secrets/vapid.json');
  let child;
  const start = async () => {
    let errors = '';
    child = spawn('sh', [join(runtime, 'dakia')], { env, stdio: ['ignore', 'ignore', 'pipe'] });
    child.stderr.on('data', b => { errors += b; });
    for (let i = 0; i < 200; i++) {
      if (child.exitCode !== null) throw new Error(`runtime exited ${child.exitCode}: ${errors.slice(0, 300)}`);
      try { if ((await fetch(`http://127.0.0.1:${port}/healthz`)).ok) return; } catch {}
      await new Promise(r => setTimeout(r, 100));
    }
    throw new Error('runtime health deadline');
  };
  const stop = async () => { if (child && child.exitCode === null) { const ended = new Promise(r => child.once('exit', r)); child.kill('SIGTERM'); await ended; } };
  const check = async () => {
    const base = `http://127.0.0.1:${port}`;
    assert.equal((await fetch(`${base}/api/push/public-key`)).status, 401);
    const login = await fetch(`${base}/auth/login`, { method: 'POST', headers: { Origin: origin }, body: new URLSearchParams({ username: 'disposable-owner', password: randomBytes(32).toString('base64url') }), redirect: 'manual' });
    assert.equal(login.status, 303);
    const cookie = login.headers.get('set-cookie').split(';')[0];
    const response = await fetch(`${base}/api/push/public-key`, { headers: { Cookie: cookie } });
    assert.equal(response.status, 200);
    const config = JSON.parse(await readFile(file, 'utf8'));
    assert.equal(await response.json(), config.publicKey);
    assert.equal((await stat(file)).mode & 0o777, 0o600);
    assert.equal((await stat(file)).uid, process.getuid());
    const worker = spawnSync(join(runtime, 'node'), [join(runtime, 'workers/push/main.mjs')], { env: { ...env, DAKIA_VAPID_FILE: file }, input: '', encoding: 'utf8' });
    assert.equal(worker.status, 0, worker.stderr);
  };
  return { start, stop, check, file };
}
const a = await instance('first'), b = await instance('second');
try {
  await a.start(); await a.check();
  const original = await readFile(a.file, 'utf8');
  await a.stop(); await a.start();
  assert.equal(await readFile(a.file, 'utf8'), original);
  await b.start(); await b.check();
  assert.notEqual(JSON.parse(await readFile(b.file, 'utf8')).publicKey, JSON.parse(original).publicKey);
  console.log('PASS: real native server fresh startup, authenticated public key, bundled push worker, private service-owned keys, restart persistence, two unique instances; no subscription or send');
} finally { await a.stop(); await b.stop(); await rm(directory, { recursive: true, force: true }); }
