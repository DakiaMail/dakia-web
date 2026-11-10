// Run after saving benchmark results, with Chrome set offline and page reloaded.
(async () => {
  const p = window.prototype;
  const checks = { online: navigator.onLine, offlineReload: window.prototypeReady && !!navigator.serviceWorker.controller };
  try { await fetch('/offline-probe'); checks.offlineNetworkBlocked = false; } catch { checks.offlineNetworkBlocked = true; }
  checks.cachedStartup = await p.load('et', true);
  checks.offlineReloadTranslation = await p.translate('et', p.fixtures[0].text);
  const a = p.manifest.models.find(m => m.source === 'et').files.model, cache = await caches.open('dakia-translation-prototype-models-v1'), url = '/models/' + a[2], original = await cache.match(url);
  try {
    await cache.put(url, new Response('corrupt model'));
    try { await p.artifact(a, true); checks.corruptPackRejected = false; } catch (e) { checks.corruptPackRejected = e.message === 'Pack integrity check failed'; }
    checks.corruptPackEvicted = !(await cache.match(url));
  } finally { await cache.put(url, original); }
  const operation = p.translate('et', Array(25).fill(p.fixtures[0].text).join('\n\n'));
  setTimeout(() => p.stop(), 20);
  try { await operation; checks.cancelled = false; } catch (e) { checks.cancelled = e.message === 'Worker stopped'; }
  await p.load('et', true); checks.recovered = await p.translate('et', 'Tere maailm');
  p.stop(); checks.complete = true; return checks;
})()
