/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
// Browser adapter for Firefox's pinned Bergamot v0.6 runtime. Based on the
// allocation/configuration in Firefox translations-engine.worker.js; no Gecko
// privileged APIs are required. Keep diagnostics from exposing message text.
import { loadBergamot } from '../../../workers/translation/browser/bergamot-translator.mjs';
self.addEventListener("unhandledrejection", event => {
  event.preventDefault(); self.postMessage({ fatal: true });
});
let bergamot, model, service;
const owned = [];
const inputBuffers = [];
const releaseBuffer = buffer => {
  inputBuffers.push(buffer);
  if (buffer.transfer) buffer.transfer();
  else structuredClone(buffer, { transfer: [buffer] });
};
async function initialize({ wasmBinary }) {
  bergamot = await new Promise((resolve, reject) => {
    const module = loadBergamot({
      INITIAL_MEMORY: 41_943_040,
      wasmBinary,
      print() {}, printErr() {},
      onAbort() { reject(new Error('runtime_initialization_failed')); },
      onRuntimeInitialized() { queueMicrotask(() => resolve(module)); },
    });
  });
  releaseBuffer(wasmBinary);
  service = new bergamot.BlockingService({ cacheSize: 0 });
}
function aligned(buffer, alignment) {
  const memory = new bergamot.AlignedMemory(buffer.byteLength, alignment);
  memory.getByteArrayView().set(new Uint8Array(buffer));
  owned.push(memory); return memory;
}
function loadTranslationModel({ from, to }, buffers) {
  if (model) throw new Error('one_model_per_worker');
  const weights = aligned(buffers.model, 256), shortlist = aligned(buffers.shortlist, 64);
  const vocabs = new bergamot.AlignedMemoryList();
  try {
    for (const vocab of buffers.vocabs) vocabs.push_back(aligned(vocab, 64));
    // Match Firefox rather than assuming a smaller workspace saves memory.
    const config = Object.entries({
      'beam-size': 1, normalize: 1.0, 'word-penalty': 0,
      'max-length-break': 128, 'mini-batch-words': 1024, workspace: 128,
      'max-length-factor': 2.0, 'skip-cost': true, 'cpu-threads': 0,
      quiet: true, 'quiet-translation': true,
      'gemm-precision': 'int8shiftAlphaAll', ...buffers.config, alignment: 'soft',
    }).map(([k,v]) => `${k}: ${v}`).join('\n') + '\n';
    model = new bergamot.TranslationModel(from, to, config, weights, shortlist, vocabs, null);
  } finally {
    vocabs.delete();
    for (const buffer of new Set([buffers.model, buffers.shortlist, ...buffers.vocabs])) releaseBuffer(buffer);
  }
}
function translate({ texts }) {
  const input = new bergamot.VectorString(), options = new bergamot.VectorResponseOptions();
  let responses;
  try {
    for (const { text, html } of texts) { input.push_back(text); options.push_back({ html, alignment: true, qualityScores: false }); }
    responses = service.translate(model, input, options);
    return texts.map((_,i) => { const response = responses.get(i); try { return { target: { text: response.getTranslatedText() } }; } finally { response.delete(); } });
  } finally { responses?.delete(); options.delete(); input.delete(); }
}
const methods = { initialize, loadTranslationModel, translate,
  __memory: () => ({ wasmHeapBytes: bergamot.HEAP8.buffer.byteLength, runtime: 'firefox-v0.6.0-wasm4', originalBuffersDetached: inputBuffers.every(b => b.byteLength === 0) }),
};
self.addEventListener('message', async ({ data: { id, name, args } }) => {
  try {
    if (!Object.hasOwn(methods, name)) throw new Error('unknown_worker_method');
    self.postMessage({ id, result: await methods[name](...args) });
  } catch (error) { self.postMessage({ id, error: { message: error.message } }); }
});
