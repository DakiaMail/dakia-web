import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
const root = path.resolve(import.meta.dirname || path.dirname(new URL(import.meta.url).pathname), '../..');
function instance() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dakia-push-bootstrap-'));
  fs.copyFileSync(path.join(root, 'deploy/native/dakia'), path.join(dir, 'dakia'));
  fs.symlinkSync(process.execPath, path.join(dir, 'node'));
  fs.symlinkSync(path.join(root, 'workers'), path.join(dir, 'workers'));
  fs.mkdirSync(path.join(dir, 'secrets'), { mode: 0o700 });
  fs.mkdirSync(path.join(dir, 'data'), { mode: 0o700 });
  fs.writeFileSync(path.join(dir, 'dakia-server'), '#!/bin/sh\nprintf "server:%s\\n" "$DAKIA_VAPID_FILE"\n', { mode: 0o755 });
  const env = { ...process.env, DAKIA_ORIGIN: 'https://mail.example.com', DAKIA_DATA_DIR: path.join(dir, 'data'), DAKIA_CREDENTIAL_KEY_FILE: path.join(dir, 'secrets/credential-key') };
  delete env.DAKIA_VAPID_FILE; delete env.DAKIA_VAPID_SUBJECT;
  return { dir, env, file: path.join(dir, 'secrets/vapid.json'), run: () => spawnSync('sh', [path.join(dir, 'dakia')], { env, encoding: 'utf8' }), cleanup: () => fs.rmSync(dir, { recursive: true, force: true }) };
}
test('existing explicit key file works without a provisioning subject and remains byte-identical', () => {
  const a = instance();
  try {
    assert.equal(a.run().status, 0);
    const original = fs.readFileSync(a.file);
    a.env.DAKIA_VAPID_FILE = path.join(a.dir, 'secrets/custom.json');
    fs.renameSync(a.file, a.env.DAKIA_VAPID_FILE);
    delete a.env.DAKIA_ORIGIN;
    const result = a.run();
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(fs.readFileSync(a.env.DAKIA_VAPID_FILE), original);
    assert(!fs.existsSync(a.file));
  } finally { a.cleanup(); }
});
test('invalid existing config fails startup without replacing keys or exposing contents', () => {
  const a = instance();
  try {
    fs.writeFileSync(a.file, '{"privateKey":"do-not-print-this-secret"}', { mode: 0o600 });
    const result = a.run();
    assert.notEqual(result.status, 0);
    assert.doesNotMatch(result.stdout, /server:/);
    assert.doesNotMatch(result.stderr, /do-not-print-this-secret/);
    assert.equal(fs.readFileSync(a.file, 'utf8'), '{"privateKey":"do-not-print-this-secret"}');
  } finally { a.cleanup(); }
});
test('concurrent provisioning publishes one complete key file without replacing it', async () => {
  const a = instance();
  try {
    const results = await Promise.all(Array.from({ length: 6 }, () => new Promise(resolve => {
      const child = spawn('sh', [path.join(a.dir, 'dakia')], { env: a.env, stdio: 'ignore' });
      child.on('exit', resolve);
    })));
    assert.deepEqual(results, [0, 0, 0, 0, 0, 0]);
    const original = fs.readFileSync(a.file, 'utf8');
    assert.equal(a.run().status, 0);
    assert.equal(fs.readFileSync(a.file, 'utf8'), original);
    assert.deepEqual(fs.readdirSync(path.dirname(a.file)), ['vapid.json']);
  } finally { a.cleanup(); }
});
for (const scenario of ['missing subject', 'invalid subject', 'missing directory', 'public permissions', 'symlink']) test(`startup fails honestly for ${scenario}`, () => {
  const a = instance();
  try {
    if (scenario === 'missing subject') delete a.env.DAKIA_ORIGIN;
    if (scenario === 'invalid subject') a.env.DAKIA_VAPID_SUBJECT = 'not-a-contact-url';
    if (scenario === 'missing directory') a.env.DAKIA_VAPID_FILE = path.join(a.dir, 'absent/vapid.json');
    if (scenario === 'public permissions' || scenario === 'symlink') {
      assert.equal(a.run().status, 0);
      if (scenario === 'public permissions') fs.chmodSync(a.file, 0o644);
      else { fs.renameSync(a.file, `${a.file}.original`); fs.symlinkSync(`${a.file}.original`, a.file); }
    }
    const result = a.run();
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /VAPID configuration failed/);
    assert.doesNotMatch(result.stdout, /server:/);
  } finally { a.cleanup(); }
});
test('both image build paths launch through the shared notification bootstrap', () => {
  for (const file of ['Dockerfile', 'deploy/Dockerfile']) {
    assert.match(fs.readFileSync(path.join(root, file), 'utf8'), /ENTRYPOINT \["sh", "\/app\/workers\/push\/start.sh", "\/app\/dakia-server"\]/);
  }
});
test('fresh native launch provisions usable private persistent per-instance keys before server startup', () => {
  const a = instance(), b = instance();
  try {
    const first = a.run();
    assert.equal(first.status, 0, first.stderr);
    assert.match(first.stdout, new RegExp(`server:${a.file}`));
    const original = fs.readFileSync(a.file, 'utf8');
    assert.equal(JSON.parse(original).subject, a.env.DAKIA_ORIGIN);
    assert.equal(fs.statSync(a.file).mode & 0o777, 0o600);
    assert.equal(fs.statSync(a.file).uid, process.getuid());
    assert.equal(a.run().status, 0);
    assert.equal(fs.readFileSync(a.file, 'utf8'), original);
    assert.equal(b.run().status, 0);
    assert.notEqual(JSON.parse(fs.readFileSync(b.file)).publicKey, JSON.parse(original).publicKey);
    const worker = spawnSync(process.execPath, [path.join(root, 'workers/push/main.mjs')], { input: '', encoding: 'utf8', env: { ...a.env, DAKIA_VAPID_FILE: a.file } });
    assert.equal(worker.status, 0, worker.stderr);
    assert.doesNotMatch(first.stdout + first.stderr, new RegExp(JSON.parse(original).privateKey));
  } finally { a.cleanup(); b.cleanup(); }
});
