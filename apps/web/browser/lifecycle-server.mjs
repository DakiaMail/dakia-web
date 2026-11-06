// Disposable two-release origin for real service-worker upgrade tests.
import { createServer } from 'node:http';
import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
for (const version of ['v1', 'v2']) {
  const result = spawnSync(process.execPath, ['../../node_modules/vite/bin/vite.js', 'build', '--outDir', `artifacts/lifecycle/${version}`], { env: { ...process.env, DAKIA_BUILD_ID: `lifecycle-${version}` }, stdio: 'inherit' });
  if (result.status !== 0) process.exit(result.status || 1);
}
let version = 'v1';
const types = { '.html': 'text/html', '.js': 'application/javascript', '.json': 'application/json', '.webmanifest': 'application/manifest+json', '.css': 'text/css', '.png': 'image/png', '.svg': 'image/svg+xml', '.woff2': 'font/woff2', '.woff': 'font/woff', '.ico': 'image/x-icon' };
createServer(async (req, res) => {
  const path = new URL(req.url, 'http://127.0.0.1:4187').pathname;
  res.setHeader('Cache-Control', 'no-store');
  if (req.method === 'POST' && /^\/__test__\/deploy\/v[12]$/.test(path)) { version = path.endsWith('v2') ? 'v2' : 'v1'; res.writeHead(204).end(); return; }
  if (path.startsWith('/api/')) { res.writeHead(404).end(); return; }
  const root = resolve(`artifacts/lifecycle/${version}`);
  let file = resolve(root, '.' + decodeURIComponent(path));
  if (file !== root && !file.startsWith(root + sep)) { res.writeHead(403).end(); return; }
  try {
    let body;
    try { body = await readFile(file); } catch { file = resolve(root, 'index.html'); body = await readFile(file); }
    res.setHeader('Content-Type', types[extname(file)] || 'application/octet-stream');
    res.writeHead(200).end(body);
  } catch { res.writeHead(500).end(); }
}).listen(4187, '127.0.0.1');
