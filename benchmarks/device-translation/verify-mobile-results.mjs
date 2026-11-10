import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const read = async path=>JSON.parse(await readFile(new URL('./results/'+path,import.meta.url)));
const environment=await read('mobile/environment.json');
assert.equal(environment.model,'SM-G780G'); assert.equal(environment.abi,'arm64-v8a');
const summary=[];
for(const source of ['et','ar','zh','ja','de']){
  const rows={};
  for(const engine of ['legacy','firefox']){
    const r=await read('mobile/'+engine+'-'+source+'.json'); rows[engine]=r;
    assert.equal(r.engine,engine); assert.equal(r.complete,true); assert.equal(r.error,undefined);
    assert.equal(r.html.structurePreserved,true);
    for(const literal of ['4821','125','test@example.test'])assert.ok(r.first.output.includes(literal));
    assert.match(r.first.output,/do not pay/i);
    for(const [i,n] of [10,5,3].entries()){
      assert.equal(r.workloads[i].samplesMs.length,n);
      assert.ok(r.workloads[i].samplesMs.every(x=>Number.isFinite(x)&&x>0));
    }
    assert.ok(r.memorySamples.length>=10);
    for(const s of r.memorySamples){
      assert.ok(s.processes.length>0);
      assert.equal(s.pssKiB,s.processes.reduce((n,p)=>n+p.pssKiB,0));
      assert.equal(s.rssKiB,s.processes.reduce((n,p)=>n+p.rssKiB,0));
    }
    assert.ok(r.physicalMemory.peakDeltaPssKiB>0);
    // No hidden forced GC or trim-memory call is needed to observe release.
    assert.ok(r.physicalMemory.releasedPssKiB < r.physicalMemory.peakPssKiB);
  }
  assert.ok(rows.firefox.memoryAfterFirst.wasmHeapBytes<rows.legacy.memoryAfterFirst.wasmHeapBytes);
  assert.ok(rows.firefox.physicalMemory.peakDeltaPssKiB<rows.legacy.physicalMemory.peakDeltaPssKiB);
  summary.push({source,baselinePeakDeltaPssMiB:rows.legacy.physicalMemory.peakDeltaPssKiB/1024,optimizedPeakDeltaPssMiB:rows.firefox.physicalMemory.peakDeltaPssKiB/1024});
}
for(const source of ['et','ja']){
  const r=await read('mobile-followup/firefox-'+source+'.json');
  assert.equal(r.complete,true);assert.equal(r.error,undefined);
  for(const key of ['idleReleased','idleStillLoadedAt10s','cancelled','blockOutputsMatch'])assert.equal(r[key],true);
  assert.equal(r.blocks,5);assert.equal(r.startup.inputBuffersDetached,true);
  assert.equal(r.memoryAfterFirst.originalBuffersDetached,true);
  assert.ok(r.firstBlockMs<r.allFiveBlocksMs);
  assert.equal(r.recovery.output,r.first.output);
  const busy=r.memorySamples.filter(x=>x.phase==='idle-wait').slice(0,3);
  const released=r.memorySamples.filter(x=>x.phase==='idle-released');
  assert.ok(busy.length>0&&released.length>0);
  assert.ok(Math.max(...released.map(x=>x.pssKiB))<Math.min(...busy.map(x=>x.pssKiB)));
}
for (const engine of ['firefox','legacy']) for (const source of ['ja','zh']) {
  const r = await read('mobile-retest/' + engine + '-' + source + '.json');
  assert.equal(r.clientVersion, 'firefox-mobile-idle-fixed-v1');
  assert.equal(r.complete, true); assert.equal(r.error, undefined);
  assert.equal(r.html.structurePreserved, true);
  assert.ok(r.workloads.flatMap(w => w.samplesMs).reduce((n,x) => n+x, 0) > 15000);
}
const idle = await read('optimized-checks-desktop.json');
assert.equal(idle.idleStillLoadedAt10s, true); assert.equal(idle.idleReleased, true);
const desktop=await read('chrome-firefox-runtime.json'), offline=await read('firefox-browser-checks.json');
assert.equal(desktop.complete,true);assert.equal(desktop.languages.length,5);
for(const row of desktop.languages)assert.equal(row.html.structurePreserved,true);
for(const key of ['offlineReload','offlineNetworkBlocked','corruptPackRejected','corruptPackEvicted','cancelled','complete'])assert.equal(offline[key],true);
const phoneOffline = await read('mobile-offline.json');
assert.equal(phoneOffline.online, false); assert.equal(phoneOffline.networkBlocked, true);
assert.equal(phoneOffline.complete, true); assert.equal(phoneOffline.rows.length, 2);
for (const row of phoneOffline.rows) {
  assert.equal(row.startup.allCacheHits, true);
  assert.equal(row.memory.originalBuffersDetached, true);
  assert.ok(row.translation.output.includes('4821'));
}
console.log('Verified physical Android A/B samples for five pairs, lower measured PSS, HTML, buffer detachment, pull-based blocks, idle release and recovery; optimized desktop/offline checks passed.');
console.log(JSON.stringify(summary,null,2));
