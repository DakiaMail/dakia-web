import { test } from 'node:test';
import assert from 'node:assert/strict';
import { version, compare, select, validate, preflight, pin, imagePrefix } from '../upgrade.mjs';
const image = `${imagePrefix}${'a'.repeat(64)}`;
const release = (tag, draft = false, prerelease = tag.includes('-')) => ({ tag_name: tag, draft, prerelease });
test('release selection prefers stable; preview numbers sort numerically', () => {
  assert.equal(select([release('v1.9.0'), release('v1.10.0'), release('v2.0.0-beta.1'), release('v3.0.0', true)]).tag_name, 'v1.10.0');
  assert.equal(select([release('v0.2.0-beta.2'), release('v0.2.0-beta.10')]).tag_name, 'v0.2.0-beta.10');
  assert.equal(select([release('v1.0.0'), release('v2.0.0-beta.1')], 'v2.0.0-beta.1').tag_name, 'v2.0.0-beta.1');
  assert.equal(select([release('sha-123'), release('v1.0.0-beta', false, false)]), undefined);
  for (const invalid of ['v01.0.0', 'v1.0.0-01', 'v1.0.0+build', 'v1.0']) assert.equal(version(invalid), null);
  assert.equal(compare('v1.0.0', 'v1.0.0-beta'), 1);
});
test('only a matching supported manifest with an immutable Dakia image is accepted', () => {
  const m = { format: 1, version: 'v1.0.0', image, revision: 'b'.repeat(40) };
  assert.deepEqual(validate(m, m.version), m);
  for (const bad of [{ format: 2 }, { version: 'v2.0.0' }, { image: 'untrusted:latest' }, { revision: 'no' }]) assert.throws(() => validate({ ...m, ...bad }, m.version));
});
test('pinning preserves other settings and rejects duplicates', () => {
  const before = '# private\nDAKIA_ORIGIN=https://mail.test\nDAKIA_GMAIL_CLIENT_SECRET=synthetic\n';
  assert.equal(pin(before, image), `${before}DAKIA_IMAGE=${image}\n`);
  assert.equal(pin(`${before}DAKIA_IMAGE=old\n`, image), `${before}DAKIA_IMAGE=${image}\n`);
  assert.throws(() => pin('DAKIA_IMAGE=a\nexport DAKIA_IMAGE=b', image));
  assert.throws(() => pin(before, 'bad\nINJECTED=true'));
});
const fixture = () => ({ config: { name: 'dakia', services: { dakia: { image: 'old', environment: {}, volumes: [{ type: 'volume', source: 'data', target: '/data' }, { type: 'bind', source: '/host/secret', target: '/secrets' }] } }, volumes: { data: { name: 'dakia_data' } } }, container: { State: { Running: true }, Image: 'sha256:old', Config: { Image: 'old', Labels: { 'com.docker.compose.project': 'dakia', 'com.docker.compose.service': 'dakia', 'com.docker.compose.project.config_files': '/host/compose.yaml', 'com.docker.compose.project.working_dir': '/host' } }, Mounts: [{ Destination: '/data', Type: 'volume', Name: 'dakia_data', Source: '/volumes/dakia_data', RW: true }, { Destination: '/secrets', Type: 'bind', Source: '/host/secret', RW: true }] } });
test('preflight resolves the actual project, image, paths and mounts', () => {
  const { config, container } = fixture();
  assert.equal(preflight(config, container, '/host').previous_image, 'sha256:old');
  for (const mutate of [c => c.name = 'other', c => c.services.dakia.image = 'new', c => c.services.dakia.environment.DAKIA_DATA_DIR = '/custom', c => c.volumes.data.name = 'empty', c => c.services.dakia.build = {}]) {
    const { config, container } = fixture(); mutate(config); assert.throws(() => preflight(config, container, '/host'));
  }
});
