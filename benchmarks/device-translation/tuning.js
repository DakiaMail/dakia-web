// Run in the prototype page with agent-browser eval --stdin < tuning.js.
(async () => {
  const NativeWorker = window.Worker; let probe;
  window.Worker = class extends NativeWorker {
    constructor() { super('/vendor/probe-worker-lowmem.js'); probe = this; }
  };
  const memory = () => new Promise((resolve, reject) => {
    const id = 987654;
    const timer = setTimeout(() => reject(new Error('Memory probe timeout')), 5000);
    const listener = ({ data }) => { if (data.id === id) { clearTimeout(timer); probe.removeEventListener('message', listener); resolve(data.result.wasmHeapBytes); } };
    probe.addEventListener('message', listener); probe.postMessage({ id, name: '__memory', args: [] });
  });
  const report = window.tuningResults = { workspaceMiB: 16, miniBatchWords: 256, languages: [], complete: false };
  try {
    for (const fixture of window.prototype.fixtures) {
      const startup = await window.prototype.load(fixture.source, true), afterLoad = await memory();
      const first = await window.prototype.translate(fixture.source, fixture.text), afterShort = await memory();
      const row = { source: fixture.source, startup, first, afterLoad, afterShort, workloads: [] };
      report.languages.push(row);
      for (const [name, repeat, n] of [['short', 1, 10], ['medium', 5, 10], ['long', 25, 3]]) {
        const text = Array(repeat).fill(fixture.text).join('\n\n'), samplesMs = [];
        for (let i = 0; i < n; i++) samplesMs.push((await window.prototype.translate(fixture.source, text)).ms);
        const sorted = [...samplesMs].sort((a,b) => a-b);
        row.workloads.push({ name, inputCharacters: text.length, samplesMs, medianMs: (sorted[Math.floor((n-1)/2)] + sorted[Math.floor(n/2)]) / 2, p95Ms: sorted[Math.ceil(n*.95)-1] });
      }
      row.afterLong = await memory(); window.prototype.stop();
    }
    report.complete = true;
  } catch (e) { report.error = e.message; }
  finally { window.Worker = NativeWorker; window.prototype.stop(); }
  return report;
})()
; 'Started';
