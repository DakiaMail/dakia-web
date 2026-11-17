import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const packageScript = path.join(root, 'scripts/package-runtime.sh');
function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dakia-native-'));
  const app = path.join(dir, 'app');
  const tools = path.join(dir, 'tools');
  fs.mkdirSync(tools);
  for (const file of ['dakia-server', 'dakia-admin', 'dakia-healthcheck', 'dakia-classifier-check', 'dakia-search-model-check', 'libonnxruntime.so.1', 'web/index.html', 'workers/translation/smoke.mjs', 'search-model/model.onnx', 'search-model/tokenizer.json', 'licenses/Node-LICENSE']) {
    const target = path.join(app, file);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, 'fixture\n', { mode: 0o755 });
  }
  fs.writeFileSync(path.join(dir, 'node'), 'fixture node\n', { mode: 0o755 });
  fs.writeFileSync(path.join(tools, 'docker'), `#!${process.execPath}\n` + `
import fs from 'node:fs';
const a = process.argv.slice(2), r = process.env.FIXTURE;
fs.appendFileSync(r + '/commands', JSON.stringify(a) + '\\n');
if (a[0] === 'image') console.log(a[3].includes('Architecture') ? process.env.ARCH : 'a'.repeat(40));
else if (a[0] === 'create') console.log('fixture-container');
else if (a[0] === 'cp') fs.cpSync(r + (a[1].endsWith('/app/.') ? '/app' : '/node'), a[2], { recursive: true });
else if (a[0] !== 'rm') process.exit(1);
`, { mode: 0o755 });
  const run = (arch = 'amd64', version = 'v0.1.0') => spawnSync('sh', [packageScript, 'fixture-image', version, arch, path.join(dir, 'out')], { encoding: 'utf8', env: { ...process.env, FIXTURE: dir, ARCH: arch, PATH: `${tools}:${process.env.PATH}` } });
  return { dir, app, run, cleanup: () => fs.rmSync(dir, { recursive: true, force: true }) };
}
for (const arch of ['amd64', 'arm64']) test(`packages the complete ${arch} runtime and a valid checksum`, () => {
  const { dir, run, cleanup } = fixture();
  try {
    const result = run(arch);
    assert.equal(result.status, 0, result.stderr);
    const out = path.join(dir, 'out');
    const archive = `dakia-v0.1.0-linux-${arch}.tar.gz`;
    const unpack = path.join(dir, 'unpack'); fs.mkdirSync(unpack);
    assert.equal(spawnSync('tar', ['-xzf', path.join(out, archive), '-C', unpack]).status, 0);
    for (const name of ['node', 'dakia', 'dakia.service', 'dakia-server', 'search-model/model.onnx', 'workers/translation/smoke.mjs', 'licenses/Node-LICENSE', 'libonnxruntime.so.1']) assert(fs.existsSync(path.join(unpack, name)), name);
    assert.equal(fs.readFileSync(path.join(unpack, 'VERSION'), 'utf8'), 'v0.1.0\n');
    assert.equal(spawnSync('shasum', ['-a', '256', '-c', `${archive}.sha256`], { cwd: out }).status, 0);
    assert.match(fs.readFileSync(path.join(dir, 'commands'), 'utf8'), /"rm","fixture-container"/);
    assert.notEqual(run(arch).status, 0, 'existing archives must not be replaced');
  } finally { cleanup(); }
});
test('also packages runtimes with statically linked ONNX', () => {
  const { app, run, cleanup } = fixture();
  try {
    fs.unlinkSync(path.join(app, 'libonnxruntime.so.1'));
    const result = run();
    assert.equal(result.status, 0, result.stderr);
  } finally { cleanup(); }
});
test('missing runtime assets fail packaging and clean up the temporary container', () => {
  const { dir, app, run, cleanup } = fixture();
  try {
    fs.unlinkSync(path.join(app, 'search-model/model.onnx'));
    assert.notEqual(run().status, 0);
    assert(!fs.existsSync(path.join(dir, 'out/dakia-v0.1.0-linux-amd64.tar.gz')));
    assert.match(fs.readFileSync(path.join(dir, 'commands'), 'utf8'), /"rm","fixture-container"/);
  } finally { cleanup(); }
});
test('rejects unsupported architectures and unsafe output versions', () => {
  const { run, cleanup } = fixture();
  try {
    assert.notEqual(run('riscv64').status, 0);
    assert.notEqual(run('amd64', '../escape').status, 0);
  } finally { cleanup(); }
});
test('launcher uses bundled assets and Node, preserving persistent operator paths', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dakia launcher '));
  try {
    fs.copyFileSync(path.join(root, 'deploy/native/dakia'), path.join(dir, 'dakia'));
    fs.writeFileSync(path.join(dir, 'dakia-server'), '#!/bin/sh\nprintf "%s\\n" "$DAKIA_NODE" "$DAKIA_WEB_ROOT" "$DAKIA_DATA_DIR" "$SQLITE_TMPDIR" "$DAKIA_LISTEN" "$LD_LIBRARY_PATH"\n', { mode: 0o755 });
    const result = spawnSync('sh', [path.join(dir, 'dakia')], { encoding: 'utf8', env: { ...process.env, DAKIA_DATA_DIR: '/private-data', DAKIA_CREDENTIAL_KEY_FILE: '/private-key', SQLITE_TMPDIR: '/private-temp', DAKIA_LISTEN: '127.0.0.1:3025' } });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(result.stdout.split('\n').slice(0, 5), [path.join(dir, 'node'), path.join(dir, 'web'), '/private-data', '/private-temp', '127.0.0.1:3025']);
    const missing = spawnSync('sh', [path.join(dir, 'dakia')], { env: { ...process.env, DAKIA_DATA_DIR: '', DAKIA_CREDENTIAL_KEY_FILE: '' } });
    assert.notEqual(missing.status, 0);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
