import { createServer } from 'node:http';
import { readFile, mkdir, writeFile, rename } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('./', import.meta.url));
const vendor = fileURLToPath(new URL('../../workers/translation/vendor/', import.meta.url));
const manifest = JSON.parse(await readFile(new URL('../../workers/translation/models.json', import.meta.url)));
const sources = ['et', 'ar', 'zh', 'ja', 'de'];
const models = manifest.models.filter(m => sources.includes(m.source));
const cache = process.env.DAKIA_PROTOTYPE_CACHE || join(tmpdir(), 'dakia-device-translation-models');
const artifacts = new Map(models.flatMap(m => [m.files.model, m.files.shortlist, ...m.files.vocabs]).map(a => [a[2], a]));
const valid = (bytes, a) => bytes.length === a[1] && createHash('sha256').update(bytes).digest('hex') === a[2];
await mkdir(cache, { recursive: true });
if (process.argv.includes('--prepare')) {
  for (const [hash, a] of artifacts) {
    const start = performance.now();
    let bytes; try { bytes = await readFile(join(cache, hash)); } catch {}
    const hit = bytes && valid(bytes, a);
    if (!hit) {
      const response = await fetch(a[0], { redirect: 'error', signal: AbortSignal.timeout(120000) });
      if (!response.ok) throw new Error(`Download failed: ${response.status}`);
      bytes = Buffer.from(await response.arrayBuffer());
      if (!valid(bytes, a)) throw new Error('Model integrity failure');
      await writeFile(join(cache, hash + '.part'), bytes);
      await rename(join(cache, hash + '.part'), join(cache, hash));
    }
    console.log(JSON.stringify({ hash, bytes: bytes.length, cacheHit: Boolean(hit), ms: performance.now() - start }));
  }
  process.exit(0);
}
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.cjs': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' };
const assets = new Map(['index.html', 'client.mjs', 'probe-worker.js', 'sw.js', 'fixtures.json', 'native.html', 'native.mjs', 'firefox-worker.js'].map(p => ['/' + p, join(root, p)]));
for (const p of ['translator-worker.cjs', 'bergamot-translator-worker.js', 'bergamot-translator-worker.wasm']) assets.set('/vendor/' + p, join(vendor, p));
assets.set('/vendor/probe-worker.js', join(root, 'probe-worker.js'));
const tunedWorker = (await readFile(join(vendor, 'translator-worker.cjs'), 'utf8')).replace('mini-batch-words: 1024', 'mini-batch-words: 256').replace('workspace: 128', 'workspace: 16');
assets.set('/vendor/probe-worker-lowmem.js', join(root, 'probe-worker-lowmem.js'));
for (const p of ['bergamot-translator.js', 'bergamot-translator.wasm']) assets.set('/firefox/' + p, join(root, 'firefox', p));
let requests = [];
const server = createServer(async (req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname;
  requests.push({ method: req.method, path });
  if (process.env.DAKIA_PROTOTYPE_ISOLATION !== '0') {
    res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
  }
  res.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'");
  try {
    if (req.method !== 'GET') { res.writeHead(405).end(); return; }
    if (path === '/vendor/translator-worker-lowmem.cjs') { res.setHeader('Content-Type', 'text/javascript'); res.end(tunedWorker); return; }
    if (path === '/manifest.json') { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify({ models })); return; }
    if (path === '/requests.json') { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify(requests)); return; }
    const hash = path.startsWith('/models/') ? path.slice(8) : null;
    const file = hash && artifacts.has(hash) ? join(cache, hash) : assets.get(path === '/' ? '/index.html' : path);
    if (!file) { res.writeHead(404).end(); return; }
    const bytes = await readFile(file);
    res.setHeader('Content-Type', hash ? 'application/octet-stream' : mime[file.slice(file.lastIndexOf('.'))] || 'application/octet-stream');
    res.setHeader('Cache-Control', 'no-cache');
    res.end(bytes);
  } catch { res.writeHead(404).end('Run node server.mjs --prepare first.'); }
});
server.listen(Number(process.env.PORT || 4178), '127.0.0.1', () => console.log('Translation prototype: http://127.0.0.1:' + server.address().port));
