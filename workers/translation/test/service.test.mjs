import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, rm, writeFile, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { normalizeHtml, ModelStore, TranslationService, validateRequest, revision } from '../service.mjs';
export const request = (source = 'et', text = 'Tere maailm', html = false) => ({
  scope: { account_id: 'account-a', message_id: 'message-a', content_revision: 1 },
  job: { id: 'job-a', account_id: 'account-a', message_id: 'message-a', content_revision: 1, model_revision: revision, status: 'running', kind: 'translate', deadline: new Date(Date.now() + 120_000).toISOString() }, source, text, html,
});

test('HTML parser normalizes real desktop Outlook fixture and strips active/remote content', () => {
  const html = '<!doctype html><html><head><style>p{color:#333}</style></head><body><table><tr><td><p>Tere esimene dokument.</p></td></tr></table></body></html><html xmlns:o="urn:schemas-microsoft-com:office:office"><head><style>.outlook{font-family:Arial}</style></head><body><table><tr><td class="outlook"><a href="https://example.test/details?x=1&amp;y=2">Tere teine dokument.</a></td></tr></table></body></html>';
  const result = normalizeHtml(html + '<script>alert(1)</script><img src="https://tracker.test"><iframe src="https://bad.test"></iframe><a onclick="bad()" href="javascript:bad()">hello</a>');
  assert.equal((result.match(/<html>/g) ?? []).length, 1);
  assert.match(result, /<table>/); assert.match(result, /class="outlook"/);
  assert.match(result, /href="https:\/\/example.test\/details\?x=1&amp;y=2"/);
  assert.doesNotMatch(result, /doctype|script|img|iframe|onclick|javascript:|alert/);
});

test('account, content/model revision, paths and deadline are rejected before inference', () => {
  assert.ok(validateRequest(request()) > 0);
  for (const change of [r => r.scope.account_id = 'other', r => r.scope.message_id = 'other', r => r.job.content_revision++, r => r.job.model_revision = 'old', r => r.job.deadline = '2000-01-01', r => r.path = '/other-user', r => r.source = '../../etc', r => r.text = 'x'.repeat(256_001)]) {
    const r = request(); change(r); assert.throws(() => validateRequest(r));
  }
});

test('download failure and corrupted cache fail closed, then recover with verified bytes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'dakia-model-test-'));
  const bytes = Buffer.from('public test weights'); const hash = createHash('sha256').update(bytes).digest('hex');
  const artifact = ['https://firefox-settings-attachments.cdn.mozilla.net/test', bytes.length, hash];
  let mode = 'bad'; let downloads = 0;
  const store = new ModelStore(root, async () => { downloads++; if (mode === 'fail') return new Response('', { status: 503 }); return new Response(mode === 'bad' ? 'invalid' : bytes); });
  try {
    await assert.rejects(store.artifact(artifact), /hash_mismatch/);
    mode = 'fail'; await assert.rejects(store.artifact(artifact), /download_failed/);
    mode = 'good'; assert.deepEqual(await store.artifact(artifact), bytes);
    await writeFile(join(root, hash), 'corrupted'); assert.deepEqual(await store.artifact(artifact), bytes);
    mode = 'fail'; assert.deepEqual(await store.artifact(artifact), bytes); assert.equal(downloads, 4);
    assert.deepEqual(await readFile(join(root, hash)), bytes);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('subprocess handles malformed job then valid protocol framing without leaking body', async () => {
  const child = spawn(process.execPath, [new URL('../main.mjs', import.meta.url).pathname], { env: { ...process.env, DAKIA_MODEL_CACHE: tmpdir() } });
  let stdout = '', stderr = ''; child.stdout.on('data', b => stdout += b); child.stderr.on('data', b => stderr += b);
  child.stdin.end('{broken}\n' + JSON.stringify({ body: 'private-mail-content' }) + '\n');
  const code = await new Promise(resolve => child.on('exit', resolve)); assert.equal(code, 0);
  assert.deepEqual(stdout.trim().split('\n').map(JSON.parse), [{ error: 'translation_failed' }, { error: 'translation_failed' }]);
  assert.equal(stderr, ''); assert.doesNotMatch(stdout, /private-mail-content/);
});
