// Local-only disposable fixture for manual Chrome review. Build the web app first.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import ts from 'typescript';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
const source = ts.transpile(fs.readFileSync(`${root}apps/web/browser/fixtures.ts`, 'utf8'), { module: ts.ModuleKind.ESNext });
const fixture = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const web = `${root}apps/web/dist`;
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.png': 'image/png', '.woff': 'font/woff', '.woff2': 'font/woff2', '.svg': 'image/svg+xml' };
http.createServer((req, res) => {
  const url = new URL(req.url, 'http://127.0.0.1:4184');
  res.setHeader('Cache-Control', 'no-store');
  if (url.pathname.startsWith('/api/')) {
    const p = url.pathname; let value = [];
    if (p === '/api/session') value = { user_id: 'disposable-review-owner', csrf_token: 'synthetic' };
    else if (p === '/api/local-access') value = { can_invite: true };
    else if (p === '/api/releases') value = { installed_version: 'v0.1.0', status: 'checked', checked_at: 1, available: { version: 'v0.2.0', revision: 'a'.repeat(40), image: 'fixture', notes_url: 'https://github.com/DakiaMail/dakia-web/releases/tag/v0.2.0' } };
    else if (p === '/api/accounts') value = fixture.accounts;
    else if (p.endsWith('/messages') || p === '/api/search') value = fixture.page;
    else if (p.endsWith('/events')) { res.setHeader('Content-Type', 'text/event-stream'); res.end(': ready\n\n'); return; }
    res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify(value)); return;
  }
  const relative = path.normalize(decodeURIComponent(url.pathname)).replace(/^\/+/, '');
  const file = path.resolve(web, relative || 'index.html');
  if (!file.startsWith(`${web}/`)) { res.writeHead(404); res.end(); return; }
  const target = fs.existsSync(file) && fs.statSync(file).isFile() ? file : `${web}/index.html`;
  res.setHeader('Content-Type', mime[path.extname(target)] || 'application/octet-stream');
  res.end(fs.readFileSync(target));
}).listen(4184, '127.0.0.1', () => console.log('Disposable UI fixture: http://127.0.0.1:4184'));
