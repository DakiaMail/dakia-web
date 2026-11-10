// The CLI runner substitutes SOURCE before evaluating this in its own test tab.
(() => {
  const source = SOURCE, p = window.prototype, fixture = p.fixtures.find(f => f.source === source);
  const r = window.mobileCase = { source, engine: p.engine, phase: 'loading', started: new Date().toISOString(), workloads: [] };
  const mark = phase => { r.phase = phase; document.querySelector('#status').textContent = p.engine + ': ' + source + ' ' + phase; };
  const memory = async () => p.memory();
  (async () => {
    r.startup = await p.load(source);
    r.memoryAfterLoad = await memory(); mark('first');
    r.first = await p.translate(source, fixture.text); r.memoryAfterFirst = await memory();
    for (const [name, repeat, n] of [['short',1,10],['medium',5,5],['long',25,3]]) {
      mark(name); const text = Array(repeat).fill(fixture.text).join('\n\n'), samplesMs = [];
      for (let i=0;i<n;i++) samplesMs.push((await p.translate(source,text)).ms);
      const sorted = [...samplesMs].sort((a,b)=>a-b);
      r.workloads.push({ name, inputCharacters: text.length, samplesMs,
        medianMs: (sorted[Math.floor((n-1)/2)]+sorted[Math.floor(n/2)])/2, p95Ms: sorted[Math.ceil(n*.95)-1] });
    }
    mark('html');
    r.html = await p.translate(source, '<html><body><table><tr><td><p>' + fixture.text + '</p></td></tr></table><a href="https://example.test/details?x=1&amp;y=2">' + fixture.text + '</a></body></html>', true);
    const parsed = new DOMParser().parseFromString(r.html.output,'text/html');
    r.html.structurePreserved = Boolean(parsed.querySelector('table tr td p')) && parsed.querySelector('a')?.getAttribute('href') === 'https://example.test/details?x=1&y=2';
    r.memoryAfterLong = await memory(); mark('retained'); r.complete = true;
    document.querySelector('#output').textContent = JSON.stringify(r,null,2);
  })().catch(e=>{r.error=e.message;r.complete=true;mark('failed');p.stop();});
  return 'Started';
})()
