import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const tools = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
function run(scenario = 'success', args = ['--yes'], before = () => {}) {
  const root = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'dakia-upgrade-mock-'));
  const bin = path.join(root, 'bin'); fs.mkdirSync(bin);
  fs.copyFileSync(path.join(tools, 'tests/fake-docker.mjs'), path.join(bin, 'docker')); fs.chmodSync(path.join(bin, 'docker'), 0o700);
  fs.writeFileSync(path.join(root, '.env'), 'DAKIA_ORIGIN=https://fixture.test\nPRIVATE_SECRET=synthetic-private\n');
  fs.writeFileSync(path.join(root, 'compose.yaml'), 'name: fixture\n');
  fs.writeFileSync(path.join(root, 'upgrade-helper.mjs'), fs.readFileSync(path.join(tools, 'upgrade.mjs'), 'utf8').replace("const root = '/installation';", `const root = ${JSON.stringify(root)};`));
  before(root);
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, UPGRADE_SCENARIO: scenario };
  for (const k of ['DAKIA_IMAGE', 'COMPOSE_FILE', 'COMPOSE_ENV_FILES', 'DOCKER_HOST']) delete env[k];
  const result = spawnSync('sh', [path.join(tools, 'upgrade.sh'), ...args], { cwd: root, env, encoding: 'utf8' });
  const state = fs.existsSync(`${root}/docker-state.json`) ? JSON.parse(fs.readFileSync(`${root}/docker-state.json`)) : { commands: [] };
  return { root, result, state, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) };
}
test('successful upgrade retains backup, pins only the image, and verifies health', () => {
  const { root, result, state, cleanup } = run();
  try {
    assert.equal(result.status, 0, result.stderr);
    assert.match(fs.readFileSync(`${root}/.env`, 'utf8'), /PRIVATE_SECRET=synthetic-private\nDAKIA_IMAGE=ghcr/);
    assert(!fs.existsSync(`${root}/.dakia-upgrades/active`));
    const backup = fs.readdirSync(`${root}/.dakia-upgrades`).find(x => /^\d/.test(x));
    assert(fs.existsSync(`${root}/.dakia-upgrades/${backup}/data/snapshot/manifest.json`));
    assert(fs.existsSync(`${root}/.dakia-upgrades/${backup}/recover.sh`));
    assert.equal(fs.readFileSync(`${root}/.dakia-upgrades/${backup}/progress`, 'utf8').trim(), 'complete');
    assert(state.commands.some(c => c[0] === 'exec'));
    assert(!result.stdout.includes('synthetic-private'));
  } finally { cleanup(); }
});
for (const scenario of ['lookup-failure', 'override', 'pull-failure', 'permissions']) test(`${scenario} never stops the installation`, () => {
  const { root, result, state, cleanup } = run(scenario);
  try { assert.notEqual(result.status, 0); assert(!state.commands.some(c => c[0] === 'compose' && c[1] === 'stop')); assert(!fs.readFileSync(`${root}/.env`, 'utf8').includes('DAKIA_IMAGE=')); } finally { cleanup(); }
});
test('backup failure restarts the unchanged container; never activates target', () => {
  const { root, result, state, cleanup } = run('backup-failure');
  try {
    assert.notEqual(result.status, 0); assert(state.commands.some(c => c[0] === 'start'));
    assert(!state.activated); assert(!fs.readFileSync(`${root}/.env`, 'utf8').includes('DAKIA_IMAGE='));
    assert(fs.existsSync(`${root}/.dakia-upgrades/active`));
  } finally { cleanup(); }
});
test('startup failure stops target and preserves backup without restarting old binary', () => {
  const { root, result, state, cleanup } = run('startup-failure');
  try {
    assert.notEqual(result.status, 0); assert(state.stopped); assert(state.activated);
    assert(!state.commands.some(c => c[0] === 'start'));
    assert(fs.existsSync(`${root}/.dakia-upgrades/active`));
    assert.match(result.stderr, /recover.sh/);
  } finally { cleanup(); }
});
for (const args of [['--check'], ['--yes']]) test(`no-op/check does not back up or stop: ${args}`, () => {
  const { result, state, cleanup } = run('noop', args);
  try { assert.equal(result.status, 0, result.stderr); assert(!state.commands.some(c => c[0] === 'compose' && c[1] === 'stop')); } finally { cleanup(); }
});
test('existing lock and unfinished active record refuse another upgrade', () => {
  for (const kind of ['lock', 'active']) {
    const { result, state, cleanup } = run('success', ['--yes'], root => {
      fs.mkdirSync(`${root}/.dakia-upgrades`);
      if (kind === 'lock') fs.mkdirSync(`${root}/.dakia-upgrades/lock`);
      else fs.writeFileSync(`${root}/.dakia-upgrades/active`, 'unfinished');
    });
    try { assert.notEqual(result.status, 0); assert(!state.commands.some(c => c[0] === 'compose' && c[1] === 'stop')); } finally { cleanup(); }
  }
});

test('default channel never downgrades; explicit downgrade fails before stopping', () => {
  for (const args of [['--yes'], ['--yes', '--version', 'v0.2.0']]) {
    const { result, state, cleanup } = run('downgrade', args);
    try { assert.equal(result.status, args.includes('--version') ? 1 : 0, result.stderr); assert(!state.commands.some(c => c[0] === 'compose' && c[1] === 'stop')); } finally { cleanup(); }
  }
});
