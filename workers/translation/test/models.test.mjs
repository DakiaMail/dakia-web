import assert from 'node:assert/strict';
import { test } from 'node:test';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { ModelStore, TranslationService, revision } from '../service.mjs';
const makeRequest = (source, text, html = false) => ({ scope: { account_id: 'a', message_id: 'm', content_revision: 1 }, job: { id: 'j', account_id: 'a', message_id: 'm', content_revision: 1, model_revision: revision, status: 'running', kind: 'translate', deadline: new Date(Date.now() + 120_000).toISOString() }, source, text, html });

test('real Linux Bergamot: four existing language fixtures, HTML and terminated-worker recovery', { skip: process.env.DAKIA_REAL_MODELS !== '1', timeout: 600_000 }, async () => {
  const service = new TranslationService(new ModelStore(process.env.DAKIA_MODEL_CACHE || join(tmpdir(), 'dakia-web-translation-models')));
  const results = [];
  for (const [source, text] of [['et', 'Tere maailm'], ['ar', 'مرحبا بالعالم'], ['zh', '你好，世界'], ['ja', 'こんにちは、世界']]) {
    const result = await service.translate(makeRequest(source, text));
    assert.notEqual(result.text.trim(), text); assert.match(result.text, /[A-Za-z]{2}/);
    assert.equal(result.account_id, 'a'); results.push({ source, input: text, output: result.text });
  }
  const html = await service.translate(makeRequest('et', '<!doctype html><html><body><table><tr><td><p>Tere esimene dokument.</p></td></tr></table></body></html><html><body><a href="https://example.test/details?x=1&amp;y=2">Tere teine dokument.</a></body></html>', true));
  assert.match(html.text, /<table>/); assert.match(html.text, /href="https:\/\/example.test\/details\?x=1&amp;y=2"/); assert.equal((html.text.match(/<html>/g) || []).length, 1);
  await assert.rejects(service.translate(makeRequest('et', 'Tere maailm'), state => { if (state.state === 'translating') void service.terminate(); }));
  const recovered = await service.translate(makeRequest('et', 'Tere maailm'));
  assert.match(recovered.text, /hello world/i);
  // A second authorized account uses a fresh WASM instance with cacheSize 0.
  const other = makeRequest('et', 'Tere maailm'); other.scope.account_id = 'b'; other.job.account_id = 'b';
  assert.equal((await service.translate(other)).account_id, 'b');
  console.log(JSON.stringify({ architecture: process.arch, platform: process.platform, node: process.version, results, html: html.text, recovery: recovered.text }));
});
