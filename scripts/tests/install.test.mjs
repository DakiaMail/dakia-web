import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const tools = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const install = path.join(tools, 'install.sh');
function run(args) {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'dakia-install-'));
  const result = spawnSync('sh', [install, ...args.map(a => a.replace('DIR', dir))], { encoding: 'utf8' });
  return { dir, result, cleanup: () => fs.rmSync(dir, { recursive: true, force: true }) };
}
test('prepare-only writes the stack and a private .env', () => {
  const { dir, result, cleanup } = run(['--prepare-only', '--domain', 'mail.example.com', '--dir', 'DIR/install']);
  try {
    assert.equal(result.status, 0, result.stderr);
    const root = path.join(dir, 'install');
    assert.equal(fs.statSync(path.join(root, '.env')).mode & 0o777, 0o600);
    assert.equal(fs.readFileSync(path.join(root, '.env'), 'utf8'), 'DAKIA_ORIGIN=https://mail.example.com\n');
    assert.match(fs.readFileSync(path.join(root, 'compose.caddy.yaml'), 'utf8'), /DAKIA_ORIGIN: \$\{DAKIA_ORIGIN:\?/);
    assert.match(fs.readFileSync(path.join(root, 'deploy/Caddyfile'), 'utf8'), /reverse_proxy dakia:3025/);
    assert(fs.existsSync(path.join(root, 'compose.yaml')));
    assert(fs.existsSync(path.join(root, 'scripts/upgrade.sh')));
    assert.match(result.stdout, /https:\/\/mail\.example\.com/);
  } finally { cleanup(); }
});
test('an existing .env is never rewritten', () => {
  const { dir, cleanup } = run(['--prepare-only', '--domain', 'mail.example.com', '--dir', 'DIR/install']);
  try {
    const env = path.join(dir, 'install/.env');
    fs.writeFileSync(env, 'DAKIA_ORIGIN=https://kept.example.com\n');
    const again = spawnSync('sh', [install, '--prepare-only', '--domain', 'other.example.com', '--dir', path.join(dir, 'install')], { encoding: 'utf8' });
    assert.equal(again.status, 0, again.stderr);
    assert.equal(fs.readFileSync(env, 'utf8'), 'DAKIA_ORIGIN=https://kept.example.com\n');
    assert.match(again.stdout, /Keeping the existing/);
  } finally { cleanup(); }
});
for (const [label, args, message] of [
  ['missing domain', ['--prepare-only'], /Set the HTTPS domain/],
  ['scheme', ['--prepare-only', '--domain', 'https://mail.example.com'], /without a scheme/],
  ['path', ['--prepare-only', '--domain', 'mail.example.com/dakia'], /without a path/],
  ['port', ['--prepare-only', '--domain', 'mail.example.com:8443'], /without a port/],
  ['not a domain', ['--prepare-only', '--domain', 'not a domain'], /Not a valid domain/],
  ['relative dir', ['--prepare-only', '--domain', 'mail.example.com', '--dir', 'relative/path'], /absolute --dir/],
  ['unknown option', ['--nope'], /Unknown option/],
]) test(`rejects ${label}`, () => {
  const { result, cleanup } = run(args);
  try {
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, message);
  } finally { cleanup(); }
});
test('help exits cleanly', () => {
  const { result, cleanup } = run(['--help']);
  try {
    assert.equal(result.status, 0);
    assert.match(result.stdout, /--prepare-only/);
  } finally { cleanup(); }
});
