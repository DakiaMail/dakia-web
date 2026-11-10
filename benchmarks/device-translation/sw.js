const NAME = 'dakia-translation-prototype-static-v5';
const ASSETS = ['/', '/index.html', '/client.mjs', '/vendor/probe-worker.js', '/firefox-worker.js', '/firefox/bergamot-translator.js', '/firefox/bergamot-translator.wasm', '/fixtures.json', '/manifest.json', '/vendor/translator-worker.cjs', '/vendor/bergamot-translator-worker.js', '/vendor/bergamot-translator-worker.wasm'];
self.addEventListener('install', event => event.waitUntil(caches.open(NAME).then(cache => cache.addAll(ASSETS)).then(() => self.skipWaiting())));
self.addEventListener('activate', event => event.waitUntil((async () => {
  for (const key of await caches.keys()) if (key.startsWith('dakia-translation-prototype-static-') && key !== NAME) await caches.delete(key);
  await self.clients.claim();
})()));
self.addEventListener('fetch', event => {
  if (event.request.method === 'GET' && ASSETS.includes(new URL(event.request.url).pathname)) event.respondWith(caches.open(NAME).then(async cache => (await cache.match(new URL(event.request.url).pathname)) || fetch(event.request)));
});
