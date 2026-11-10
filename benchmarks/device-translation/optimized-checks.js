(async () => {
  const p = window.prototype, results = { engine: p.engine };
  if (p.engine !== 'firefox') throw new Error('Select the Firefox runtime');
  results.startup = await p.load('et', true);
  results.detachedAfterLoad = (await p.memory()).originalBuffersDetached;
  const fixture = p.fixtures.find(f=>f.source==='et');
  const blocks = Array(5).fill({ text: fixture.text });
  const started = performance.now(), iterator = p.translateBlocks('et', blocks);
  const first = await iterator.next();
  results.firstBlockMs = performance.now() - started;
  results.firstBlock = first.value.output;
  results.firstIndex = first.value.index;
  const outputs = [first.value.output];
  for await (const chunk of iterator) outputs.push(chunk.output);
  results.allFiveBlocksMs = performance.now() - started;
  results.blocks = outputs.length;
  results.blockOutputsMatch = outputs.every(x=>x===outputs[0]);
  const controller = new AbortController(), cancelled = p.translateBlocks('et', blocks, { signal: controller.signal });
  await cancelled.next(); controller.abort();
  try { await cancelled.next(); results.cancelled = false; } catch(e) { results.cancelled = e.name==='AbortError'; }
  results.memoryBeforeIdle = await p.memory();
  results.idleStart = new Date().toISOString();
  // The caller/OS sampler observes release over these 16 seconds.
  await new Promise(resolve=>setTimeout(resolve,10000));
  results.idleStillLoadedAt10s = p.state().loaded;
  await new Promise(resolve=>setTimeout(resolve,6000));
  try { await p.memory(); results.idleReleased=false; } catch { results.idleReleased=true; }
  await p.load('et',true); results.recovery = await p.translate('et','Tere maailm'); p.stop();
  results.complete=true;
  window.optimizedChecks=results;
  document.querySelector('#status').textContent='Optimized checks complete';
  document.querySelector('#output').textContent=JSON.stringify(results,null,2);
  return results;
})()
