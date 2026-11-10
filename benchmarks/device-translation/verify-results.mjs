import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const read = async name => JSON.parse(await readFile(new URL('./results/' + name, import.meta.url)));
const online = await read('chrome-online.json'), offline = await read('chrome-offline.json'), checks = await read('browser-checks.json');
for (const report of [online, offline]) {
  assert.equal(report.complete, true); assert.equal(report.error, undefined);
  assert.equal(report.languages.length, 5);
  for (const row of report.languages) {
    assert.equal(row.html.structurePreserved, true);
    for (const literal of ['4821', '125', 'test@example.test']) assert.ok(row.first.output.includes(literal));
    assert.match(row.first.output, /do not pay/i);
    assert.equal(row.cachedFreshWorker.length, 3);
    assert.ok(row.cachedFreshWorker.every(x => x.allCacheHits));
    for (const workload of row.workloads) {
      assert.equal(workload.samplesMs.length, workload.name === 'long' ? 3 : 10);
      assert.ok(workload.samplesMs.every(ms => Number.isFinite(ms) && ms > 0));
    }
  }
}
assert.equal(offline.online, false); assert.equal(offline.cacheOnly, true);
for (const row of offline.languages) {
  assert.equal(row.startup.allCacheHits, true);
  assert.equal(row.first.output, online.languages.find(x => x.source === row.source).first.output);
}
for (const key of ['offlineReload', 'offlineNetworkBlocked', 'corruptPackRejected', 'corruptPackEvicted', 'cancelled', 'complete']) assert.equal(checks[key], true, key);
assert.match(checks.recovered.output, /hello world/i);
const tuned = await read('chrome-low-workspace.json');
assert.equal(tuned.complete, true);
for (const row of tuned.languages) assert.equal(row.first.output, online.languages.find(x => x.source === row.source).first.output);
const nonisolated = await read('nonisolated.json');
assert.equal(nonisolated.crossOriginIsolated, false);
assert.equal(nonisolated.rows.length, 2);
for (const row of nonisolated.rows) assert.equal(row.translation.output, online.languages.find(x => x.source === row.source).first.output);
console.log('Verified real online/offline results for five language pairs, preserved HTML, cache integrity, cancellation and recovery.');
