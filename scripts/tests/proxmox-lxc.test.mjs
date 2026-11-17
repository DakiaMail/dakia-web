import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('../../ct/dakia.sh', import.meta.url));
const installer = fileURLToPath(new URL('../../install/dakia-install.sh', import.meta.url));
function fixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dakia-community-'));
  const tools = path.join(dir, 'tools'); fs.mkdirSync(tools);
  fs.mkdirSync(path.join(dir, 'core'));
  fs.writeFileSync(path.join(dir, 'vms.json'), '[]');
  const release = { tag_name: 'v1.2.3', draft: false, prerelease: false, assets: ['dakia-v1.2.3-linux-amd64.tar.gz', 'SHA256SUMS'].map(name => ({ name, state: 'uploaded', size: 100 })) };
  fs.writeFileSync(path.join(dir, 'release.json'), JSON.stringify(release));
  fs.writeFileSync(path.join(dir, 'core/build.func'), `
header_info() { :; }
variables() { :; }
color() { :; }
catch_errors() { set -e; }
ensure_whiptail() { :; }
msg_info() { :; }
msg_ok() { :; }
msg_error() { printf '%s\\n' "$*" >&2; }
_cs_fetch_text() { cat "$FIXTURE/upgrader.py"; }
start() { echo start >> "$FIXTURE/events"; if [[ "\${TEST_UPDATE:-}" == 1 ]]; then update_script; exit; fi; }
build_container() { printf '%s\\n' "$var_dakia_release" "$var_dakia_arch" "$var_dakia_origin" "$var_os" "$var_version" "$var_unprivileged" "$var_nesting" "$var_cpu" "$var_ram" "$var_disk" > "$FIXTURE/build"; }
description() { :; }
`);
  const fake = `#!${process.execPath}
import fs from 'node:fs';
import path from 'node:path';
const name = path.basename(process.argv[1]), a = process.argv.slice(2), root = process.env.FIXTURE;
fs.appendFileSync(root + '/commands', JSON.stringify([name, ...a]) + '\\n');
if (name === 'uname') console.log('x86_64');
if (name === 'pvesh') process.stdout.write(fs.readFileSync(root + '/vms.json'));
if (name === 'curl') process.stdout.write(fs.readFileSync(root + '/release.json'));
if (name === 'whiptail') { process.stderr.write(process.env.ORIGIN_REPLY || 'https://mail.example.com'); process.exit(Number(process.env.DIALOG_STATUS || '0')); }
`;
  for (const name of ['pveversion', 'pvesh', 'curl', 'uname', 'whiptail']) fs.writeFileSync(path.join(tools, name), fake, { mode: 0o755 });
  return {
    dir, release,
    run: (env = {}) => spawnSync('bash', [script], { encoding: 'utf8', env: { ...process.env, FIXTURE: dir, COMMUNITY_SCRIPTS_CORE_DIR: dir, var_dakia_origin: 'https://mail.example.com', var_ctid: '900', PHS_SILENT: '1', ...env, PATH: `${tools}:${process.env.PATH}` } }),
    built: () => fs.existsSync(path.join(dir, 'build')),
    cleanup: () => fs.rmSync(dir, { recursive: true, force: true }),
  };
}
test('delegates menu and creation to the framework, exporting the selected release and safe defaults', () => {
  const f = fixture();
  try {
    const r = f.run(); assert.equal(r.status, 0, r.stderr);
    assert.equal(fs.readFileSync(path.join(f.dir, 'events'), 'utf8'), 'start\n');
    assert.deepEqual(fs.readFileSync(path.join(f.dir, 'build'), 'utf8').trim().split('\n'), ['v1.2.3', 'amd64', 'https://mail.example.com', 'ubuntu', '24.04', '1', '0', '2', '4096', '20']);
  } finally { f.cleanup(); }
});
for (const type of ['qemu', 'lxc']) test(`refuses explicitly requested existing ${type} ID before framework creation`, () => {
  const f = fixture();
  try {
    fs.writeFileSync(path.join(f.dir, 'vms.json'), JSON.stringify([{ vmid: 900, type }]));
    const r = f.run(); assert.notEqual(r.status, 0); assert.match(r.stderr, /already exists/); assert(!f.built());
    assert(!fs.existsSync(path.join(f.dir, 'events')));
  } finally { f.cleanup(); }
});
for (const kind of ['missing bundle', 'prerelease', 'unsafe tag', 'missing checksum']) test(`refuses ${kind} before building a guest`, () => {
  const f = fixture();
  try {
    if (kind === 'missing bundle') f.release.assets = f.release.assets.filter(a => a.name === 'SHA256SUMS');
    if (kind === 'missing checksum') f.release.assets = f.release.assets.filter(a => a.name !== 'SHA256SUMS');
    if (kind === 'prerelease') f.release.prerelease = true;
    if (kind === 'unsafe tag') f.release.tag_name = '../bad';
    fs.writeFileSync(path.join(f.dir, 'release.json'), JSON.stringify(f.release));
    assert.notEqual(f.run().status, 0); assert(!f.built());
  } finally { f.cleanup(); }
});
test('refuses unsupported guest OS', () => {
  const f = fixture();
  try {
    assert.notEqual(f.run({ var_os: 'debian' }).status, 0); assert(!f.built());
  } finally { f.cleanup(); }
});
test('framework update dispatch runs the native upgrader and propagates failure', () => {
  const f = fixture();
  try {
    fs.writeFileSync(path.join(f.dir, 'upgrader.py'), 'import os, pathlib\npathlib.Path(os.environ["FIXTURE"], "updated").touch()\n');
    const result = f.run({ TEST_UPDATE: '1' });
    assert.equal(result.status, 0, result.stderr);
    assert(fs.existsSync(path.join(f.dir, 'updated'))); assert(!f.built());
    fs.writeFileSync(path.join(f.dir, 'upgrader.py'), 'raise SystemExit(1)\n');
    assert.notEqual(f.run({ TEST_UPDATE: '1' }).status, 0); assert(!f.built());
  } finally { f.cleanup(); }
});
test('unattended runs require an explicit origin', () => {
  const f = fixture();
  try { const r = f.run({ var_dakia_origin: '' }); assert.notEqual(r.status, 0); assert.match(r.stderr, /Set var_dakia_origin/); assert(!f.built()); }
  finally { f.cleanup(); }
});
test('the origin dialog forwards its answer and cancel exits without creating a guest', () => {
  const f = fixture();
  try {
    const r = f.run({ var_dakia_origin: '', PHS_SILENT: '0', ORIGIN_REPLY: 'https://other.example.com' });
    assert.equal(r.status, 0, r.stderr);
    assert.equal(fs.readFileSync(path.join(f.dir, 'build'), 'utf8').trim().split('\n')[2], 'https://other.example.com');
    fs.unlinkSync(path.join(f.dir, 'build'));
    assert.equal(f.run({ var_dakia_origin: '', PHS_SILENT: '0', DIALOG_STATUS: '1' }).status, 0);
    assert(!f.built());
  } finally { f.cleanup(); }
});
// Exercise the actual unprivileged download stage, redirecting only its work directory.
const download = fs.readFileSync(installer, 'utf8').split("<<'DOWNLOAD'\n")[1].split('\nDOWNLOAD')[0]
  .replace('cd /var/lib/dakia/download', 'cd "$DOWNLOAD_FIXTURE"');
for (const kind of ['valid', 'corrupt', 'missing', 'duplicate']) test(`download stage handles ${kind} checksums`, () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dakia-download-'));
  try {
    const archive = 'dakia-v1.2.3-linux-amd64.tar.gz';
    fs.writeFileSync(path.join(dir, 'payload'), 'native archive bytes');
    const digest = createHash('sha256').update('native archive bytes').digest('hex');
    const line = `${kind === 'corrupt' ? '0'.repeat(64) : digest}  ${archive}\n`;
    fs.writeFileSync(path.join(dir, 'manifest'), kind === 'missing' ? `${digest}  other.tar.gz\n` : line.repeat(kind === 'duplicate' ? 2 : 1));
    fs.writeFileSync(path.join(dir, 'curl'), `#!${process.execPath}
import fs from 'node:fs';
const a = process.argv.slice(2), root = process.env.DOWNLOAD_FIXTURE;
fs.copyFileSync(root + (a.some(v => v.endsWith('/SHA256SUMS')) ? '/manifest' : '/payload'), a[a.indexOf('-o') + 1]);
`, { mode: 0o755 });
    const r = spawnSync('sh', ['-s', '--', 'v1.2.3', 'amd64'], { input: download, encoding: 'utf8', env: { ...process.env, DOWNLOAD_FIXTURE: dir, PATH: `${dir}:${process.env.PATH}` } });
    if (kind === 'valid') assert.equal(r.status, 0, r.stderr);
    else assert.notEqual(r.status, 0);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
for (const origin of ['http://mail.example.com', 'https://mail.example.com/', 'https://user@mail.example.com', 'https://@mail.example.com', 'https://mail.example.com\nINJECTED=yes', 'https://mail.example.com?q=x', 'https://[::1]bad']) test(`rejects unsafe origin ${JSON.stringify(origin)}`, () => {
  const f = fixture();
  try { assert.notEqual(f.run({ var_dakia_origin: origin }).status, 0); assert(!f.built()); }
  finally { f.cleanup(); }
});
