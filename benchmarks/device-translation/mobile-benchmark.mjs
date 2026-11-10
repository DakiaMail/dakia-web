// Controls only the disposable prototype Chrome tab. ADB provides Android
// process PSS/RSS independently of JavaScript and WASM heap counters.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
const exec = promisify(execFile);
const session = process.env.DAKIA_MOBILE_SESSION || 'dakia-mobile';
const origin = process.env.DAKIA_MOBILE_URL || 'http://127.0.0.1:4197';
if (!/^http:\/\/127\.0\.0\.1:\d+$/.test(origin)) throw new Error('Use a dedicated loopback prototype origin');
const followup = process.env.DAKIA_MOBILE_FOLLOWUP === '1';
const directory = new URL(followup ? './results/mobile-followup/' : process.env.DAKIA_MOBILE_RETEST === '1' ? './results/mobile-retest/' : './results/mobile/', import.meta.url);
await mkdir(directory, { recursive: true });
const command = async (file,args) => (await exec(file,args,{maxBuffer:8*1024*1024,timeout:30000})).stdout.trim();
const adb = (...args) => command('adb',args);
const ab = (...args) => command('agent-browser',['--session',session,...args]);
const evaluate = async script => JSON.parse(await ab('eval',script));
const version = await (await fetch('http://127.0.0.1:9223/json/version')).json();
const socket = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject;});
let serial=0; const waiting=new Map();
socket.onmessage=({data})=>{const m=JSON.parse(data);const p=waiting.get(m.id);if(p){clearTimeout(p.timer);waiting.delete(m.id);m.error?p.reject(new Error(m.error.message)):p.resolve(m.result);}};
const cdp=(method,params={})=>new Promise((resolve,reject)=>{const id=++serial;const timer=setTimeout(()=>{waiting.delete(id);reject(new Error('CDP timeout'));},10000);waiting.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));});
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function memory(phase, retry = 0) {
  const started=performance.now();
  const {processInfo}=await cdp('SystemInfo.getProcessInfo');
  const pids=processInfo.map(p=>p.id).filter(n=>Number.isSafeInteger(n)&&n>0);
  const raw=await adb('shell',`for pid in ${pids.join(' ')}; do dumpsys meminfo "$pid"; done`);
  const processes=[];
  for(const section of raw.split('** MEMINFO in pid ').slice(1)) {
    const pid=Number(section.match(/^\d+/)?.[0]);
    const totals=section.match(/TOTAL PSS:\s*(\d+)\s+TOTAL RSS:\s*(\d+)\s+TOTAL SWAP PSS:\s*(\d+)/);
    if(totals)processes.push({pid,type:processInfo.find(p=>p.id===pid)?.type,pssKiB:+totals[1],rssKiB:+totals[2],swapPssKiB:+totals[3]});
  }
  if(processes.length!==pids.length) {
    if (retry < 3) return memory(phase, retry + 1);
    throw new Error('Incomplete Android process memory sample after retries');
  }
  return {at:new Date().toISOString(),phase,retry,sampleDurationMs:performance.now()-started,processes,
    pssKiB:processes.reduce((n,p)=>n+p.pssKiB,0),rssKiB:processes.reduce((n,p)=>n+p.rssKiB,0),swapPssKiB:processes.reduce((n,p)=>n+p.swapPssKiB,0)};
}
const template=await readFile(new URL(followup ? './mobile-followup-case.js' : './mobile-case.js',import.meta.url),'utf8');
const environment={date:new Date().toISOString(),model:await adb('shell','getprop','ro.product.model'),android:await adb('shell','getprop','ro.build.version.release'),abi:await adb('shell','getprop','ro.product.cpu.abi'),meminfo:await adb('shell','cat','/proc/meminfo'),browser:version.Browser,batteryBefore:await adb('shell','dumpsys','battery'),thermalBefore:await adb('shell','dumpsys','thermalservice'),memoryMethod:'Sum of Android dumpsys meminfo PSS/RSS for every Chrome process returned by CDP SystemInfo.getProcessInfo; shared RSS is counted multiple times. Includes existing Chrome tabs. Baseline is the same test tab without a worker. PSS is the primary physical-memory metric.'};
await writeFile(new URL('environment.json',directory),JSON.stringify(environment,null,2)+'\n');
const sources=(process.env.DAKIA_MOBILE_SOURCES||'et,ar,zh,ja,de').split(',');
const engines=(process.env.DAKIA_MOBILE_ENGINES||'legacy,firefox').split(',');
try {
  for(const engine of engines) for(const source of sources) {
    if(!['legacy','firefox'].includes(engine)||!['et','ar','zh','ja','de'].includes(source))throw new Error('Unknown benchmark variant');
    const cooldownStart = performance.now();
    if (process.env.DAKIA_MOBILE_COOLDOWN === '1') {
      while (!/Thermal Status: 0(?:\s|$)/.test(await adb('shell','dumpsys','thermalservice'))) {
        if (performance.now() - cooldownStart > 240000) throw new Error('Phone did not cool to thermal status 0');
        console.log(JSON.stringify({cooling:true,engine,source,waitedSeconds:Math.round((performance.now()-cooldownStart)/1000)}));
        await sleep(10000);
      }
    }
    const cooldownWaitMs = performance.now() - cooldownStart;
    const thermalBeforeCase = await adb('shell','dumpsys','thermalservice');
    await ab('open',origin+'/?engine='+engine);
    await ab('wait','--fn','window.prototypeReady === true');
    // Refresh this standalone prototype's static assets, retaining model packs.
    await evaluate('(async()=>{prototype.stop();const r=await navigator.serviceWorker.getRegistration();await r?.unregister();for(const k of await caches.keys())if(k.startsWith("dakia-translation-prototype-static-"))await caches.delete(k);return true;})()');
    await ab('reload');
    await ab('wait','--fn','window.prototypeReady === true');
    const metadata=await evaluate('({clientVersion:prototype.version,engine:prototype.engine,userAgent:navigator.userAgent,hardwareConcurrency:navigator.hardwareConcurrency,deviceMemoryGiB:navigator.deviceMemory})');
    if (metadata.clientVersion !== 'firefox-mobile-idle-fixed-v1') throw new Error('Stale prototype client');
    const samples=[];
    for(let i=0;i<3;i++)samples.push(await memory('baseline'));
    await evaluate(template.replace('const source = SOURCE', 'const source = ' + JSON.stringify(source)));
    let result;
    const deadline=performance.now()+240000;
    do {
      const state=await evaluate('({phase:window.mobileCase.phase,complete:!!window.mobileCase.complete})');
      samples.push(await memory(state.phase));
      if(state.complete){result=await evaluate('window.mobileCase');break;}
      if(performance.now()>deadline)throw new Error('Case deadline exceeded');
      await sleep(300);
    }while(true);
    if(result.error)throw new Error(engine+' '+source+': '+result.error);
    for(let i=0;i<3;i++)samples.push(await memory('retained'));
    await evaluate('(prototype.stop(), true)');
    for(let i=0;i<3;i++){await sleep(300);samples.push(await memory('released'));}
    const median=xs=>{const s=xs.sort((a,b)=>a-b);return s[Math.floor(s.length/2)];};
    const baseline=median(samples.filter(x=>x.phase==='baseline').map(x=>x.pssKiB));
    const retained=median(samples.filter(x=>x.phase==='retained').map(x=>x.pssKiB));
    const peak=Math.max(...samples.map(x=>x.pssKiB));
    const report={...metadata,...result,cooldownWaitMs,thermalBeforeCase,memorySamples:samples,physicalMemory:{baselinePssKiB:baseline,peakPssKiB:peak,retainedPssKiB:retained,peakDeltaPssKiB:peak-baseline,retainedDeltaPssKiB:retained-baseline,releasedPssKiB:median(samples.filter(x=>x.phase==='released').map(x=>x.pssKiB))},batteryAfter:await adb('shell','dumpsys','battery'),thermalAfter:await adb('shell','dumpsys','thermalservice')};
    await writeFile(new URL(engine+'-'+source+'.json',directory),JSON.stringify(report,null,2)+'\n');
    console.log(JSON.stringify({engine,source,shortMs:result.workloads?.[0]?.medianMs,longMs:result.workloads?.[2]?.medianMs,wasmMiB:result.memoryAfterFirst?.wasmHeapBytes/1048576,peakDeltaPssMiB:(peak-baseline)/1024}));
  }
} finally {try { await evaluate('(prototype.stop(), true)'); } catch {} socket.close();}
