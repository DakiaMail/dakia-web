import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

test('static-only worker never intercepts private, external, navigation or unlisted requests', async () => {
  const handlers = new Map(), deleted = [], put = [];
  let claimed = false, work;
  const source = readFileSync(new URL('../../../apps/web/public/sw.js', import.meta.url), 'utf8')
    .replace('/* DAKIA_STATIC_ASSETS */ []', JSON.stringify(['/assets/app-123.js']));
  const cache = { keys: async () => [{ url: 'https://mail.example.test/assets/old.js' }], delete: async key => deleted.push(key.url),
    match: async () => undefined, put: async (key, value) => put.push(key) };
  runInNewContext(source, {
    self: { location: { origin: 'https://mail.example.test' }, addEventListener: (name, handler) => handlers.set(name, handler), clients: { claim: async () => { claimed = true; } } },
    caches: { keys: async () => ['dakia-shell-v1', 'other-app'], delete: async key => deleted.push(key), open: async () => cache },
    fetch: async () => ({ ok: true, headers: new Headers({ 'content-type': 'application/javascript' }), clone() { return this; } }), URL,
  });
  handlers.get('activate')({ waitUntil: promise => { work = promise; } }); await work;
  assert.deepEqual(deleted, ['dakia-shell-v1', 'https://mail.example.test/assets/old.js']);
  assert.equal(claimed, true);
  for (const [path, method] of [['/api/accounts', 'GET'], ['/auth/login', 'GET'], ['/?account=x&message=y', 'GET'], ['/assets/unknown.js', 'GET'], ['/assets/app-123.js?private=1', 'GET'], ['/assets/app-123.js', 'POST'], ['https://external.test/assets/app-123.js', 'GET']]) {
    let intercepted = false;
    handlers.get('fetch')({ request: { url: new URL(path, 'https://mail.example.test').href, method }, respondWith: () => { intercepted = true; } });
    assert.equal(intercepted, false, path);
  }
  handlers.get('fetch')({ request: { url: 'https://mail.example.test/assets/app-123.js', method: 'GET' }, respondWith: promise => { work = promise; } }); await work;
  assert.deepEqual(put, ['/assets/app-123.js']);
});


test('a waiting worker activates only for an explicit same-origin app request', async () => {
  const handlers = new Map();
  let activations = 0, work;
  runInNewContext(readFileSync(new URL('../../../apps/web/public/sw.js', import.meta.url), 'utf8'), {
    self: {
      addEventListener: (name, handler) => handlers.set(name, handler),
      location: { origin: 'https://mail.example.test' },
      skipWaiting: async () => { activations++; },
    }, URL,
  });
  const message = handlers.get('message');
  for (const event of [
    { data: { type: 'OTHER' }, source: { url: 'https://mail.example.test/' } },
    { data: { type: 'DAKIA_SKIP_WAITING' } },
    { data: { type: 'DAKIA_SKIP_WAITING' }, source: { url: 'https://outside.example.test/' } },
    { data: { type: 'DAKIA_SKIP_WAITING' }, source: { url: 'invalid' } },
  ]) message(event);
  assert.equal(activations, 0);
  message({ data: { type: 'DAKIA_SKIP_WAITING' }, source: { url: 'https://mail.example.test/' }, waitUntil: promise => { work = promise; } });
  await work;
  assert.equal(activations, 1);
});
