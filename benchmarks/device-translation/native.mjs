const fixtures = await (await fetch('/fixtures.json')).json();
const button = document.querySelector('#run'), status = document.querySelector('#status'), output = document.querySelector('#output');
button.onclick = async () => {
  if (button.getAttribute('aria-disabled') === 'true') return;
  const source = document.querySelector('#source').value, fixture = fixtures.find(f => f.source === source);
  const report = window.nativeResults = { source, userAgent: navigator.userAgent, date: new Date().toISOString(), online: navigator.onLine, progress: [], samples: [] };
  const paint = () => { output.textContent = JSON.stringify(report, null, 2); };
  const started = performance.now(); let translator, timer;
  button.setAttribute('aria-disabled', 'true'); status.textContent = 'Preparing language pack';
  const controller = new AbortController();
  try {
    if (!('Translator' in self)) throw new Error('Translator API unavailable');
    // create() is called directly in the click handler, before any await,
    // because pack acquisition can require transient user activation.
    const creating = Translator.create({ sourceLanguage: source, targetLanguage: 'en', signal: controller.signal, monitor(m) { m.addEventListener('downloadprogress', e => { report.progress.push({ loaded: e.loaded, ms: performance.now() - started }); paint(); }); } });
    translator = await Promise.race([creating, new Promise((_,reject) => { timer = setTimeout(() => { controller.abort(); reject(new Error('Pack acquisition exceeded 60 seconds')); }, 60000); })]);
    clearTimeout(timer); report.createMs = performance.now() - started;
    status.textContent = 'Translating';
    const firstStart = performance.now(); report.firstOutput = await translator.translate(fixture.text); report.firstMs = performance.now() - firstStart;
    for (const [name, repeat, n] of [['short',1,10],['medium',5,10],['long',25,3]]) {
      const text = Array(repeat).fill(fixture.text).join('\n\n'), samplesMs = [];
      for (let i=0;i<n;i++) { const begin = performance.now(); await translator.translate(text); samplesMs.push(performance.now()-begin); }
      const sorted = [...samplesMs].sort((a,b)=>a-b);
      report.samples.push({ name, inputCharacters: text.length, samplesMs, medianMs: (sorted[Math.floor((n-1)/2)] + sorted[Math.floor(n/2)]) / 2, p95Ms: sorted[Math.ceil(n*.95)-1] });
    }
    report.complete = true; status.textContent = 'Benchmark complete';
  } catch (e) { report.error = e.name + ': ' + e.message; status.textContent = 'Test failed'; }
  finally { clearTimeout(timer); translator?.destroy(); report.elapsedMs = performance.now()-started; button.removeAttribute('aria-disabled'); paint(); }
};
window.nativeReady = true;
