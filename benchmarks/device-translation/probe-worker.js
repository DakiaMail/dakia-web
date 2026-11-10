// Suppress upstream diagnostics, which can contain translated content.
console.log = console.warn = console.error = () => {};
self.addEventListener('message', event => {
  if (event.data.name !== '__memory') return;
  event.stopImmediatePropagation();
  self.postMessage({ id: event.data.id, result: { wasmHeapBytes: Module.HEAP8.buffer.byteLength } });
});
importScripts('/vendor/translator-worker.cjs');
