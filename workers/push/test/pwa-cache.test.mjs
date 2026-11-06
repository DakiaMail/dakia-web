import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

test('network-only PWA never intercepts or persists requests and removes only obsolete Dakia shell caches', async () => {
  const handlers = new Map(), deleted = [];
  let claimed = false, work;
  runInNewContext(readFileSync(new URL('../../../apps/web/public/sw.js', import.meta.url), 'utf8'), {
    self: { addEventListener: (name, handler) => handlers.set(name, handler), clients: { claim: async () => { claimed = true; } } },
    caches: {
      keys: async () => ['dakia-shell-v1', 'dakia-shell-v2', 'other-app'],
      delete: async key => { deleted.push(key); return true; },
      open: () => { throw new Error('No cache creation or writing allowed'); },
    }, URL,
  });
  assert.equal(handlers.has('fetch'), false);
  assert.equal(handlers.has('install'), false);
  assert.equal(handlers.has('push'), true);
  assert.equal(handlers.has('notificationclick'), true);
  handlers.get('activate')({ waitUntil: promise => { work = promise; } });
  await work;
  assert.deepEqual(deleted, ['dakia-shell-v1', 'dakia-shell-v2']);
  assert.equal(claimed, true);
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
