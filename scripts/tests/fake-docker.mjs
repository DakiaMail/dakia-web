#!/usr/bin/env node
// Narrow Docker CLI double. The real helper still validates and writes configuration.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
const args = process.argv.slice(2), root = process.cwd();
const scenario = process.env.UPGRADE_SCENARIO;
const stateFile = path.join(root, 'docker-state.json');
const state = fs.existsSync(stateFile) ? JSON.parse(fs.readFileSync(stateFile)) : { stopped: false, activated: false, commands: [] };
state.commands.push(args);
const save = () => fs.writeFileSync(stateFile, JSON.stringify(state));
const result = text => { if (text !== undefined) console.log(text); save(); process.exit(0); };
const fail = () => { save(); process.exit(1); };
const old = `sha256:${'a'.repeat(64)}`, next = `sha256:${'b'.repeat(64)}`;
const image = `ghcr.io/dakiamail/dakia-web@${next}`;
const configured = () => scenario === 'override' ? 'old' : process.env.DAKIA_IMAGE || /^DAKIA_IMAGE=(.+)$/m.exec(fs.readFileSync('.env', 'utf8'))?.[1] || 'old';
const mounts = [{ Type: 'volume', Name: 'fixture_data', Source: '/volumes/fixture_data', Destination: '/data', RW: true }, { Type: 'volume', Name: 'fixture_secrets', Source: '/volumes/fixture_secrets', Destination: '/secrets', RW: true }];
if (args[0] === 'context') result('unix:///fake.sock');
if (args[0] === 'compose') {
  if (args[1] === 'version') result('Docker Compose');
  if (args[1] === 'ps') result(state.activated ? 'new-container' : 'old-container');
  if (args[1] === 'config') result(JSON.stringify({ name: 'fixture', services: { dakia: { image: configured(), volumes: [{ type: 'volume', source: 'data', target: '/data' }, { type: 'volume', source: 'secrets', target: '/secrets' }] } }, volumes: { data: { name: 'fixture_data' }, secrets: { name: 'fixture_secrets' } } }));
  if (args[1] === 'stop') { state.stopped = true; result(); }
  if (args[1] === 'up') { state.activated = true; state.stopped = false; result(); }
}
if (args[0] === 'inspect') {
  if (args.includes('--format')) {
    if (args[2] === '{{.Image}}') result(args.at(-1) === 'old-container' ? old : next);
    result(scenario === 'startup-failure' ? 'exited unhealthy' : 'running healthy');
  }
  result(JSON.stringify([{ Image: old, State: { Running: true }, Config: { Image: 'old', Labels: { 'com.docker.compose.project': 'fixture', 'com.docker.compose.service': 'dakia', 'com.docker.compose.project.config_files': `${root}/compose.yaml`, 'com.docker.compose.project.working_dir': root } }, Mounts: mounts }]));
}
if (args[0] === 'image') {
  if (args[1] === 'tag') result();
  if (args.includes('{{.Id}}')) result(scenario === 'noop' ? old : next);
  result(scenario === 'downgrade' ? 'v0.3.0' : 'v0.1.0');
}
if (args[0] === 'pull') { if (scenario === 'pull-failure') fail(); result(); }
if (args[0] === 'start') { state.stopped = false; result(); }
if (args[0] === 'exec') result();
if (args[0] === 'run') {
  const helper = args.indexOf('/tool/upgrade.mjs');
  if (helper >= 0) {
    const command = args[helper + 1];
    if (command === 'check-mounts') { if (scenario === 'permissions') fail(); result(); }
    if (command === 'discover') {
      if (scenario === 'lookup-failure') fail();
      fs.writeFileSync('.dakia-upgrades/work/target.json', JSON.stringify({ format: 1, version: 'v0.2.0', revision: 'b'.repeat(40), image, notes_url: 'https://github.com/DakiaMail/dakia-web/releases/tag/v0.2.0' })); result();
    }
    if (command === 'backup-files') {
      const mount = args.find(a => a.endsWith(',dst=/upgrade'));
      const backup = mount.slice('type=bind,src='.length, -',dst=/upgrade'.length);
      fs.mkdirSync(`${backup}/data`); fs.mkdirSync(`${backup}/secrets`); fs.writeFileSync(`${backup}/secrets/credential-key`, Buffer.alloc(32, 1)); result();
    }
    const run = spawnSync(process.execPath, [path.join(root, 'upgrade-helper.mjs'), ...args.slice(helper + 1)], { encoding: 'utf8' });
    process.stdout.write(run.stdout); process.stderr.write(run.stderr); save(); process.exit(run.status ?? 1);
  }
  if (args.includes('/app/dakia-admin')) {
    if (scenario === 'backup-failure') fail();
    const mount = args.find(a => a.endsWith(',dst=/upgrade'));
    const backup = mount.slice('type=bind,src='.length, -',dst=/upgrade'.length);
    fs.mkdirSync(`${backup}/snapshot`); fs.writeFileSync(`${backup}/snapshot/manifest.json`, '{}'); result();
  }
  if (args.includes('--input-type=module')) {
    const i = args.indexOf('-e');
    const run = spawnSync(process.execPath, ['--input-type=module', '-e', args[i + 1].replace('/tool/upgrade.mjs', path.join(root, 'upgrade-helper.mjs')), ...args.slice(i + 2)], { encoding: 'utf8' });
    process.stdout.write(run.stdout); process.stderr.write(run.stderr); save(); process.exit(run.status ?? 1);
  }
}
console.error('Unexpected Docker call', args[0]); fail();
