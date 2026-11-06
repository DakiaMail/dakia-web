import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { payloadFor } from '../service.mjs';

function fixture(fetcher = async () => { throw Error('offline'); }) {
  const handlers = new Map(), visible = new Map(), requests = [], opened = [];
  const self = {
    location: { origin: 'https://dakia.example.test' },
    addEventListener: (name, fn) => handlers.set(name, fn),
    registration: { showNotification: async (title, options) => {
      assert.equal(options.renotify, false);
      visible.set(options.tag, { title, ...options });
    } },
    clients: { matchAll: async () => [], openWindow: async url => opened.push(url) },
  };
  runInNewContext(readFileSync(new URL('../../../apps/web/public/sw.js', import.meta.url), 'utf8'), {
    self, URL, AbortController, setTimeout, clearTimeout,
    fetch: async (url, options) => { requests.push({ url, options }); return fetcher(url, options); },
  });
  async function dispatch(name, event) {
    let pending;
    handlers.get(name)({ ...event, waitUntil: p => { pending = p; } });
    await pending;
  }
  return { self, visible, requests, opened,
    push: data => dispatch('push', { data: { json: () => data } }),
    click: path => dispatch('notificationclick', { notification: { close() {}, data: { path } } }),
  };
}
const job = { id: 'a:m:s', account_id: 'a', message_id: 'm' };

test('production payload and SW enrich safe text, dedupe retries, separate messages/accounts and click exact message', async () => {
  const f = fixture(async () => ({ ok: true, json: async () => ({ sender: '<b>Alice</b>\u202e', subject: 'Hello\nworld', preview: '<script>secret()</script><p>Preview</p>', account: 'work@example.test' }) }));
  const payload = JSON.parse(payloadFor(job));
  assert.deepEqual(Object.keys(payload).sort(), ['tag', 'url']);
  await f.push(payload); await f.push(payload);
  assert.equal(f.visible.size, 1);
  const n = f.visible.get('dakia:a:m:s');
  assert.equal(n.title, 'Alice · Hello world');
  assert.equal(n.body, 'work@example.test\nPreview');
  assert.equal(n.data.path, '/?account=a&message=m');
  assert.equal(f.requests[0].url, '/api/accounts/a/messages/m/notification');
  assert.equal(f.requests[0].options.credentials, 'same-origin');
  assert.equal(f.requests[0].options.cache, 'no-store');
  assert.equal(f.requests[0].options.redirect, 'error');
  await f.push(JSON.parse(payloadFor({ ...job, id: 'b:m:s', account_id: 'b' })));
  await f.push(JSON.parse(payloadFor({ ...job, id: 'a:n:s', message_id: 'n' })));
  assert.equal(f.visible.size, 3);
  await f.click(n.data.path);
  assert.equal(f.opened[0], 'https://dakia.example.test/?account=a&message=m');
});

test('offline, signed-out, removed message, malformed data and empty fields have useful fallbacks', async () => {
  for (const fetcher of [async () => { throw Error('offline'); }, async () => ({ ok: false }), async () => ({ ok: true, json: async () => { throw Error('bad JSON'); } })]) {
    const f = fixture(fetcher);
    await f.push(JSON.parse(payloadFor(job)));
    assert.equal(f.visible.get('dakia:a:m:s').title, 'New mail');
    assert.equal(f.visible.get('dakia:a:m:s').data.path, '/?account=a&message=m');
  }
  const f = fixture(async () => ({ ok: true, json: async () => ({}) }));
  await f.push({ url: '/accounts/a/messages/m', tag: 'dakia:legacy' });
  assert.equal(f.visible.get('dakia:legacy').title, 'Unknown sender · (No subject)');
  assert.equal(f.visible.get('dakia:legacy').data.path, '/?account=a&message=m');
  for (const url of ['https://evil.test/?account=a&message=m', '/auth/logout', '/?account=a', 'https://[', undefined]) {
    const count = f.requests.length;
    await f.push({ url, tag: 'x'.repeat(900), title: 'untrusted', body: 'untrusted' });
    assert.equal(f.requests.length, count);
    assert.equal(f.visible.get('dakia-new-mail').title, 'New mail');
    assert.equal(f.visible.get('dakia-new-mail').data.path, '/');
  }
  await f.push(null);
  await f.click('https://evil.test/');
  assert.equal(f.opened.at(-1), 'https://dakia.example.test/');
});

test('summary deadline aborts stalled background fetch and still shows generic notification', async () => {
  let aborted = false;
  const f = fixture((url, { signal }) => new Promise((resolve, reject) => {
    signal.addEventListener('abort', () => { aborted = true; reject(Error('timeout')); });
  }));
  await f.push(JSON.parse(payloadFor(job)));
  assert.equal(aborted, true);
  assert.equal(f.visible.get('dakia:a:m:s').title, 'New mail');
});

test('bounded Unicode text and identifier round trips; click navigation failure opens a new client', async () => {
  const f = fixture(async () => ({ ok: true, json: async () => ({ sender: '😀'.repeat(500), subject: 's'.repeat(500), preview: 'p'.repeat(500) }) }));
  const data = JSON.parse(payloadFor({ id: 'roundtrip', account_id: 'a /?#&', message_id: 'm /?#&' }));
  await f.push(data);
  const n = f.visible.get('dakia:roundtrip');
  assert.equal(Array.from(n.title).length, 203);
  assert.equal(n.body.length, 160);
  assert.equal(new URL(n.data.path, f.self.location.origin).searchParams.get('message'), 'm /?#&');
  f.self.clients.matchAll = async () => [{ url: f.self.location.origin, navigate: async () => { throw Error('closed'); } }];
  await f.click(n.data.path);
  assert.equal(f.opened.length, 1);
  let focused = false;
  f.self.clients.matchAll = async () => [{ url: f.self.location.origin, navigate: async url => { assert.equal(url, f.opened[0]); return { focus: async () => { focused = true; } }; } }];
  await f.click(n.data.path);
  assert.equal(focused, true);
  assert.equal(f.opened.length, 1);
});
