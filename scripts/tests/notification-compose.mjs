// Disposable Compose acceptance, including read-only rootfs and UID 1000.
// Usage: node scripts/tests/notification-compose.mjs IMAGE
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
const directory = await mkdtemp(join(tmpdir(), 'dakia-compose-push-'));
const image = process.argv[2];
assert(image);
const override = join(directory, 'override.yaml');
await writeFile(override, `services:\n  dakia:\n    image: ${image}\n    network_mode: none\n    ports: !reset []\n`);
const projects = [`dakia-push-a-${process.pid}`, `dakia-push-b-${process.pid}`];
const env = { ...process.env, DAKIA_ORIGIN: 'https://mail.test', DAKIA_IMAGE: image, DAKIA_RELEASE_CHECKS: 'false', DAKIA_PORT: '0' };
delete env.DAKIA_VAPID_FILE; delete env.DAKIA_VAPID_SUBJECT;
function compose(project, ...args) {
  const result = spawnSync('docker', ['compose', '-p', project, '-f', resolve('compose.yaml'), '-f', override, ...args], { env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}
const check = `
const assert = require('node:assert/strict'), fs = require('node:fs'), crypto = require('node:crypto');
(async () => {
const file = '/secrets/vapid.json';
const config = JSON.parse(fs.readFileSync(file));
assert.equal(config.subject, 'https://mail.test');
assert.equal(fs.statSync(file).mode & 511, 384);
assert.equal(fs.statSync(file).uid, 1000);
assert.equal(process.getuid(), 1000);
const login = await fetch('http://127.0.0.1:3025/auth/login', { method: 'POST', headers: { Origin: 'https://mail.test' }, body: new URLSearchParams({username:'disposable-owner', password:'disposable-local-test-password-12345'}), redirect:'manual' });
assert.equal(login.status,303);
const response = await fetch('http://127.0.0.1:3025/api/push/public-key', {headers:{Cookie:login.headers.get('set-cookie').split(';')[0]}});
assert.equal(response.status,200);
assert.equal(await response.json(),config.publicKey);
console.log(crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'));
})().catch(e => {console.error(e.message);process.exit(1)});
`;
try {
  const hashes = [];
  for (const project of projects) {
    compose(project, 'up', '-d', '--wait', '--wait-timeout', '90');
    hashes.push(compose(project, 'exec', '-T', 'dakia', 'node', '-e', check));
    compose(project, 'exec', '-T', 'dakia', 'sh', '-c', 'DAKIA_VAPID_FILE=/secrets/vapid.json node /app/workers/push/main.mjs </dev/null');
  }
  assert.notEqual(hashes[0], hashes[1]);
  compose(projects[0], 'restart');
  compose(projects[0], 'up', '-d', '--wait', '--wait-timeout', '90');
  assert.equal(compose(projects[0], 'exec', '-T', 'dakia', 'node', '-e', check), hashes[0]);
  const failed = spawnSync('docker', ['run', '--rm', '--network', 'none', '--read-only', '-e', 'DAKIA_ORIGIN=https://mail.test', '-e', 'DAKIA_VAPID_FILE=/app/forbidden.json', image], { encoding: 'utf8' });
  assert.notEqual(failed.status, 0);
  assert.match(failed.stderr, /VAPID configuration failed/);
  console.log('PASS: actual Compose fresh instances healthy; authenticated push key ready; UID1000/mode0600; two distinct persistent keys; restart unchanged; packaged worker starts; read-only write failure reported; network disabled, no sends');
} finally {
  for (const project of projects) compose(project, 'down', '--volumes', '--remove-orphans');
  await rm(directory, { recursive: true, force: true });
}
