// Run inside the final image, without production data or network access.
import assert from 'node:assert/strict';
import { Worker } from 'node:worker_threads';
import { normalizeHtml, ModelStore, TranslationService, revision } from './service.mjs';

assert.match(normalizeHtml('<p>Tere maailm</p>'), /<p>Tere maailm<\/p>/);
const worker = new Worker(new URL('./vendor/translator-worker.cjs', import.meta.url), { stdout: true, stderr: true });
worker.stdout.resume(); worker.stderr.resume();
try {
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('runtime_smoke_timeout')), 15_000);
    const finish = error => { clearTimeout(timer); error ? reject(error) : resolve(); };
    worker.once('error', finish);
    worker.once('exit', () => finish(new Error('runtime_smoke_exit')));
    worker.once('message', response => finish(response.error ? new Error('runtime_smoke_failed') : null));
    worker.postMessage({ id: 1, name: 'initialize', args: [{ cacheSize: 0, useNativeIntGemm: false }] });
  });
} finally { await worker.terminate(); }

if (process.argv.includes('--models')) {
  const scope = { account_id: 'smoke', message_id: 'smoke', content_revision: 1 };
  const job = { ...scope, id: 'smoke', model_revision: revision, kind: 'translate', status: 'running', deadline: new Date(Date.now() + 120_000).toISOString() };
  const result = await new TranslationService(new ModelStore(process.env.DAKIA_MODEL_CACHE)).translate({ scope, job, source: 'et', text: '<p>Tere maailm</p>', html: true });
  assert.match(result.text, /hello world/i);
}
console.log('Translation runtime smoke passed');
