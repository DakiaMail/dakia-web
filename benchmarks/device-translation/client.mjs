const status = document.querySelector('#status'), output = document.querySelector('#output');
const manifest = await (await fetch('/manifest.json')).json();
const fixtures = await (await fetch('/fixtures.json')).json();
await navigator.serviceWorker.register('/sw.js');
await navigator.serviceWorker.ready;
if (!navigator.serviceWorker.controller) await new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, { once: true }));
const CACHE = 'dakia-translation-prototype-models-v1';
const engine = new URL(location.href).searchParams.get('engine') === 'legacy' ? 'legacy' : 'firefox';
let idleTimer;
let worker, pending = new Map(), serial = 0;
const digest = async b => [...new Uint8Array(await crypto.subtle.digest('SHA-256', b))].map(b => b.toString(16).padStart(2, '0')).join('');
async function artifact(a, cacheOnly) {
  const url = '/models/' + a[2], cache = await caches.open(CACHE);
  let response = await cache.match(url), hit = Boolean(response);
  if (!response) {
    if (cacheOnly) throw new Error('Pack is not saved');
    response = await fetch(url);
    if (!response.ok) throw new Error('Pack download failed');
  }
  const bytes = await response.arrayBuffer();
  if (bytes.byteLength !== a[1] || await digest(bytes) !== a[2]) {
    await cache.delete(url); throw new Error('Pack integrity check failed');
  }
  if (!hit) await cache.put(url, new Response(bytes));
  return { bytes, hit };
}
function stop() {
  clearTimeout(idleTimer);
  worker?.terminate(); worker = null;
  for (const p of pending.values()) { clearTimeout(p.timer); p.reject(new Error('Worker stopped')); }
  pending.clear();
}
function startWorker() {
  stop(); worker = new Worker(engine === 'firefox' ? '/firefox-worker.js' : '/vendor/probe-worker.js');
  worker.onmessage = ({ data }) => {
    const p = pending.get(data.id); if (!p) return;
    clearTimeout(p.timer); pending.delete(data.id);
    if (pending.size === 0) idleTimer = setTimeout(stop, 15_000);
    data.error ? p.reject(new Error(data.error.message)) : p.resolve(data.result);
  };
  worker.onerror = () => stop();
}
function callWithTransfers(name, args, transfer = []) {
  if (!worker) return Promise.reject(new Error('Worker not loaded'));
  clearTimeout(idleTimer);
  return new Promise((resolve, reject) => {
    const id = ++serial;
    const timer = setTimeout(() => { reject(new Error('Worker deadline exceeded')); stop(); }, 120000);
    pending.set(id, { resolve, reject, timer });
    worker.postMessage({ id, name, args }, transfer);
  });
}
function call(name, ...args) { return callWithTransfers(name, args); }
async function load(source, cacheOnly = false) {
  const m = manifest.models.find(m => m.source === source), begin = performance.now();
  if (!m) throw new Error('Unsupported source language');
  stop();
  const data = [];
  for (const a of [m.files.model, m.files.shortlist, ...m.files.vocabs]) data.push(await artifact(a, cacheOnly));
  const packMs = performance.now() - begin;
  startWorker(); const initStart = performance.now();
  await call('initialize', { cacheSize: 0, useNativeIntGemm: false });
  const runtimeInitMs = performance.now() - initStart, modelStart = performance.now();
  const packBytes = data.reduce((n, x) => n + x.bytes.byteLength, 0);
  const buffers = { model: data[0].bytes, shortlist: data[1].bytes, vocabs: data.slice(2).map(x => x.bytes), config: m.config };
  const transfer = engine === 'firefox' ? [...new Set(data.map(x => x.bytes))] : [];
  await callWithTransfers('loadTranslationModel', [{ from: source, to: 'en' }, buffers], transfer);
  return { packMs, runtimeInitMs, modelLoadMs: performance.now() - modelStart, totalMs: performance.now() - begin, inputBuffersDetached: engine === 'firefox' && data.every(x => x.bytes.byteLength === 0), allCacheHits: data.every(x => x.hit), packBytes };
}
async function translate(source, text, html = false) {
  const begin = performance.now();
  const result = await call('translate', { models: [{ from: source, to: 'en' }], texts: [{ text, html, qualityScores: false }] });
  return { ms: performance.now() - begin, output: result[0].target.text, inputCharacters: text.length, inputUtf8Bytes: new TextEncoder().encode(text).length };
}
const summary = values => { const s = [...values].sort((a,b) => a-b); return { n: s.length, medianMs: (s[Math.floor((s.length - 1) / 2)] + s[Math.floor(s.length / 2)]) / 2, p95Ms: s[Math.ceil(s.length * .95) - 1], minMs: s[0], maxMs: s.at(-1) }; };
const paint = result => { output.textContent = JSON.stringify(result, null, 2); };
async function chromeAvailability() {
  const result = { exposed: 'Translator' in self, languages: {} };
  if (result.exposed) for (const { source } of fixtures) {
    try { result.languages[source] = await Translator.availability({ sourceLanguage: source, targetLanguage: 'en' }); }
    catch (e) { result.languages[source] = e.name; }
  }
  return result;
}
async function run({ cacheOnly = false, repetitions = 10 } = {}) {
  const report = window.results = { date: new Date().toISOString(), engine, userAgent: navigator.userAgent, hardwareConcurrency: navigator.hardwareConcurrency, deviceMemoryGiB: navigator.deviceMemory, crossOriginIsolated, online: navigator.onLine, cacheOnly, repetitions, chrome: await chromeAvailability(), languages: [], longTasks: [], heartbeatMaxDriftMs: 0 };
  let previous = performance.now();
  const heartbeat = setInterval(() => { const now = performance.now(); report.heartbeatMaxDriftMs = Math.max(report.heartbeatMaxDriftMs, now - previous - 20); previous = now; }, 20);
  const observer = new PerformanceObserver(list => report.longTasks.push(...list.getEntries().map(x => ({ durationMs: x.duration, startMs: x.startTime }))));
  observer.observe({ type: 'longtask', buffered: false });
  try {
    for (const fixture of fixtures) {
      status.textContent = 'Testing ' + fixture.source + ' to English';
      const startup = await load(fixture.source, cacheOnly);
      const first = await translate(fixture.source, fixture.text);
      const result = { source: fixture.source, startup, first, reference: fixture.reference, workloads: [] };
      report.languages.push(result);
      for (const [name, repeats, count] of [['short', 1, repetitions], ['medium', 5, repetitions], ['long', 25, 3]]) {
        const text = Array(repeats).fill(fixture.text).join('\n\n'), samples = [];
        for (let i = 0; i < count; i++) samples.push(await translate(fixture.source, text));
        result.workloads.push({ name, inputCharacters: text.length, inputUtf8Bytes: samples[0].inputUtf8Bytes, ...summary(samples.map(x => x.ms)), samplesMs: samples.map(x => x.ms), outputCharacters: samples[0].output.length });
      }
      const html = '<html><body><table><tr><td><p>' + fixture.text + '</p></td></tr></table><a href="https://example.test/details?x=1&amp;y=2">' + fixture.text + '</a></body></html>';
      result.html = await translate(fixture.source, html, true);
      const parsed = new DOMParser().parseFromString(result.html.output, 'text/html');
      result.html.structurePreserved = Boolean(parsed.querySelector('table tr td p')) && parsed.querySelector('a')?.getAttribute('href') === 'https://example.test/details?x=1&y=2';
      result.memory = await call('__memory');
      const cold = [];
      for (let i = 0; i < 3; i++) { const setup = await load(fixture.source, true); const translation = await translate(fixture.source, fixture.text); cold.push({ ...setup, firstTranslationMs: translation.ms, readyToResultMs: setup.totalMs + translation.ms }); }
      result.cachedFreshWorker = cold;
      stop(); paint(report);
    }
    report.complete = true;
    status.textContent = cacheOnly ? 'Saved-pack test complete' : 'Benchmark complete';
  } catch (e) { report.error = e.message; status.textContent = 'Test failed: ' + e.message; }
  finally { clearInterval(heartbeat); observer.disconnect(); stop(); paint(report); }
  return report;
}
// Pull-based blocks let a reader request visible paragraphs first. Callers can
// stop consuming when content is no longer visible, without queuing the rest.
async function* translateBlocks(source, blocks, { signal } = {}) {
  for (let index = 0; index < blocks.length; index++) {
    signal?.throwIfAborted();
    const block = blocks[index];
    // Each block must be plain text or independently valid sanitized HTML.
    const result = await translate(source, block.text, Boolean(block.html));
    signal?.throwIfAborted();
    yield { index, ...result };
  }
}
window.prototype = { version: 'firefox-mobile-idle-fixed-v1', state: () => ({ loaded: Boolean(worker), pending: pending.size }), run, load, translate, stop, translateBlocks, artifact, manifest, fixtures, chromeAvailability, engine, memory: () => call('__memory') };
async function launch(cacheOnly) {
  const buttons = [...document.querySelectorAll('button')];
  // aria-disabled keeps keyboard focus on the activating button.
  buttons.slice(0,2).forEach(b => b.setAttribute('aria-disabled', 'true'));
  try { await run({ cacheOnly }); } finally { buttons.forEach(b => b.removeAttribute('aria-disabled')); }
}
document.querySelector('#run').onclick = e => { if (e.currentTarget.getAttribute('aria-disabled') !== 'true') void launch(false); };
document.querySelector('#offline').onclick = e => { if (e.currentTarget.getAttribute('aria-disabled') !== 'true') void launch(true); };
document.querySelector('#export').onclick = () => {
  const url = URL.createObjectURL(new Blob([JSON.stringify(window.results || {}, null, 2)], { type: 'application/json' }));
  const a = document.createElement('a'); a.href = url; a.download = 'translation-results.json'; a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
};
window.prototypeReady = true;
