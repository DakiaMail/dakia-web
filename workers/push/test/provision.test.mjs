import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const helper = new URL('../provision.mjs', import.meta.url);

test('provisioning requires an explicit subject and preserves existing private keys', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'dakia-push-provision-'));
  const file = join(directory, 'vapid.json');
  const env = { ...process.env, DAKIA_VAPID_FILE: file };
  delete env.DAKIA_VAPID_SUBJECT;
  try {
    const missing = spawnSync(process.execPath, [helper.pathname], { env, encoding: 'utf8' });
    assert.notEqual(missing.status, 0);
    assert.match(missing.stderr, /DAKIA_VAPID_SUBJECT required/);
    await assert.rejects(readFile(file), { code: 'ENOENT' });
    env.DAKIA_VAPID_SUBJECT = 'https://example.com';
    const created = spawnSync(process.execPath, [helper.pathname], { env, encoding: 'utf8' });
    assert.equal(created.status, 0);
    const original = await readFile(file, 'utf8');
    const parsed = JSON.parse(original);
    assert.equal(parsed.subject, env.DAKIA_VAPID_SUBJECT);
    assert.ok(parsed.publicKey && parsed.privateKey);
    assert.equal((await stat(file)).mode & 0o777, 0o600);
    env.DAKIA_VAPID_SUBJECT = 'https://example.org';
    const preserved = spawnSync(process.execPath, [helper.pathname], { env, encoding: 'utf8' });
    assert.equal(preserved.status, 0);
    assert.equal(await readFile(file, 'utf8'), original);
    assert.doesNotMatch(created.stdout + preserved.stdout, new RegExp(parsed.privateKey));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
