import { createHash } from 'node:crypto';
import { mkdir, readFile, rename, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { Worker } from 'node:worker_threads';
import { parse, serialize } from 'parse5';
import sanitize from 'sanitize-html';

export const manifest = JSON.parse(await readFile(new URL('./models.json', import.meta.url), 'utf8'));
export const revision = createHash('sha256').update(await readFile(new URL('./models.json', import.meta.url))).digest('hex');
const clean = html => sanitize(html, {
  allowedTags: ['html', 'head', 'body', 'p', 'br', 'div', 'span', 'b', 'strong', 'i', 'em', 'u', 's', 'blockquote', 'pre', 'code', 'ul', 'ol', 'li', 'table', 'tbody', 'thead', 'tr', 'td', 'th', 'a', 'h1', 'h2', 'h3', 'h4'],
  allowedAttributes: { a: ['href', 'title'], '*': ['class', 'lang', 'dir'], td: ['colspan', 'rowspan'], th: ['colspan', 'rowspan'] },
  allowedSchemes: ['https', 'http', 'mailto'], allowProtocolRelative: false,
});
export const normalizeHtml = html => serialize(parse(clean(html)));
export const verify = (bytes, [url, size, hash]) => bytes.length === size && createHash('sha256').update(bytes).digest('hex') === hash;

// Only public model weights enter this process-wide cache. Paths/URLs come
// exclusively from the pinned manifest and operator-selected root, never jobs.
export class ModelStore {
  constructor(root, fetcher = fetch) { this.root = root; this.fetcher = fetcher; }
  async artifact(artifact, progress = () => {}, signal) {
    const [url, size, hash] = artifact;
    if (!/^[a-f0-9]{64}$/.test(hash) || !Number.isSafeInteger(size) || size < 1 || size > 150_000_000 || !url.startsWith('https://firefox-settings-attachments.cdn.mozilla.net/')) throw new Error('invalid_model_manifest');
    await mkdir(this.root, { recursive: true, mode: 0o700 });
    const path = join(this.root, hash);
    try { const bytes = await readFile(path); if (verify(bytes, artifact)) return bytes; } catch (error) { if (error.code !== 'ENOENT') throw error; }
    progress({ state: 'downloading', artifact: hash, downloaded: 0, total: size });
    const response = await this.fetcher(url, { signal, redirect: 'error' });
    if (!response.ok) throw new Error('model_download_failed');
    const chunks = []; let received = 0;
    for await (const chunk of response.body) {
      received += chunk.length;
      if (received > size) throw new Error('model_size_mismatch');
      chunks.push(chunk); progress({ state: 'downloading', artifact: hash, downloaded: received, total: size });
    }
    const bytes = Buffer.concat(chunks);
    if (!verify(bytes, artifact)) throw new Error('model_hash_mismatch');
    const temporary = `${path}.${process.pid}.part`;
    try { await writeFile(temporary, bytes, { mode: 0o600 }); await rename(temporary, path); }
    finally { await rm(temporary, { force: true }); }
    return bytes;
  }
  async load(source, progress, signal) {
    const model = manifest.models.find(m => m.source === source && m.target === 'en');
    if (!model) throw new Error('unsupported_language');
    const array = b => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
    const modelBytes = await this.artifact(model.files.model, progress, signal);
    const shortlist = await this.artifact(model.files.shortlist, progress, signal);
    const vocabs = [];
    for (const artifact of model.files.vocabs) vocabs.push(array(await this.artifact(artifact, progress, signal)));
    return { model: array(modelBytes), shortlist: array(shortlist), vocabs, config: model.config };
  }
}

export function validateRequest(request, now = Date.now()) {
  const { job, scope, source, text, html } = request;
  if (!job || !scope || typeof scope.account_id !== 'string' || !scope.account_id || typeof scope.message_id !== 'string' || !scope.message_id
      || job.account_id !== scope.account_id || job.message_id !== scope.message_id || job.content_revision !== scope.content_revision
      || !Number.isSafeInteger(scope.content_revision) || scope.content_revision < 0 || job.kind !== 'translate' || job.status !== 'running'
      || job.model_revision !== revision || typeof job.id !== 'string' || !job.id || typeof text !== 'string' || Buffer.byteLength(text) > 256_000 || typeof html !== 'boolean'
      || !manifest.models.some(m => m.source === source && m.target === 'en')
      || Object.keys(request).some(k => !['job', 'scope', 'source', 'text', 'html'].includes(k))) throw new Error('invalid_or_stale_job');
  const deadline = Date.parse(job.deadline);
  if (!Number.isFinite(deadline) || deadline <= now) throw new Error('deadline_expired');
  return Math.min(deadline - now, 120_000);
}

export class TranslationService {
  constructor(store) { this.store = store; this.busy = false; this.worker = null; }
  async terminate() { if (this.worker) await this.worker.terminate(); }
  async translate(request, progress = () => {}) {
    const timeout = validateRequest(request);
    if (this.busy) throw new Error('worker_busy');
    this.busy = true;
    const controller = new AbortController();
    let timer;
    const operation = async () => {
      const buffers = await this.store.load(request.source, progress, controller.signal);
      controller.signal.throwIfAborted();
      const worker = this.worker = new Worker(new URL('./vendor/translator-worker.cjs', import.meta.url), { stdout: true, stderr: true });
      // Upstream diagnostics can contain content. Do not forward to logs/stdout.
      worker.stdout.resume(); worker.stderr.resume();
      let serial = 0; const pending = new Map();
      const fail = () => { for (const p of pending.values()) p.reject(new Error('worker_failed')); pending.clear(); };
      worker.on('error', fail); worker.on('exit', fail);
      worker.on('message', ({ id, result, error }) => { const p = pending.get(id); if (!p) return; pending.delete(id); error ? p.reject(new Error('translation_failed')) : p.resolve(result); });
      const call = (name, ...args) => new Promise((resolve, reject) => { const id = ++serial; pending.set(id, { resolve, reject }); worker.postMessage({ id, name, args }); });
      await call('initialize', { cacheSize: 0, useNativeIntGemm: false });
      await call('loadTranslationModel', { from: request.source, to: 'en' }, buffers);
      progress({ state: 'translating' });
      const text = request.html ? normalizeHtml(request.text) : request.text;
      const result = await call('translate', { models: [{ from: request.source, to: 'en' }], texts: [{ text, html: request.html, qualityScores: false }] });
      validateRequest(request);
      if (typeof result?.[0]?.target?.text !== 'string') throw new Error('invalid_worker_response');
      return { job_id: request.job.id, account_id: request.scope.account_id, message_id: request.scope.message_id, content_revision: request.scope.content_revision, model_revision: revision, text: request.html ? normalizeHtml(result[0].target.text) : result[0].target.text };
    };
    try {
      return await Promise.race([operation(), new Promise((_, reject) => { timer = setTimeout(() => { controller.abort(); reject(new Error('deadline_expired')); }, timeout); })]);
    } finally {
      clearTimeout(timer); controller.abort();
      await this.terminate(); this.worker = null; this.busy = false;
    }
  }
}
