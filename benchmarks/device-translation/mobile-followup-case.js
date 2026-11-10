(() => {
  const source = SOURCE, p = prototype, fixture = p.fixtures.find(f=>f.source===source);
  const r = window.mobileCase = { source, engine:p.engine, phase:'loading', started:new Date().toISOString() };
  const mark = phase=>{r.phase=phase;document.querySelector('#status').textContent=source+' '+phase;};
  (async()=>{
    r.startup=await p.load(source,true); r.memoryAfterLoad=await p.memory();
    const blocks=Array(5).fill({text:fixture.text});
    mark('blocks'); const started=performance.now(),iterator=p.translateBlocks(source,blocks);
    const first=await iterator.next();r.firstBlockMs=performance.now()-started;r.first=first.value;
    const outputs=[first.value.output];
    for await(const chunk of iterator)outputs.push(chunk.output);
    r.allFiveBlocksMs=performance.now()-started;r.blocks=outputs.length;r.blockOutputsMatch=outputs.every(x=>x===outputs[0]);
    const controller=new AbortController(),cancelled=p.translateBlocks(source,blocks,{signal:controller.signal});
    await cancelled.next();controller.abort();
    try{await cancelled.next();r.cancelled=false;}catch(e){r.cancelled=e.name==='AbortError';}
    r.memoryAfterFirst=await p.memory();mark('idle-wait');r.idleStart=new Date().toISOString();
    await new Promise(resolve=>setTimeout(resolve,10000));
    r.idleStillLoadedAt10s=p.state().loaded;
    await new Promise(resolve=>setTimeout(resolve,6000));
    try{await p.memory();r.idleReleased=false;}catch(e){r.idleReleased=e.message==='Worker not loaded';}
    mark('idle-released'); await new Promise(resolve=>setTimeout(resolve,3000));
    mark('recovery');await p.load(source,true);r.recovery=await p.translate(source,fixture.text);p.stop();
    r.complete=true;mark('retained');document.querySelector('#output').textContent=JSON.stringify(r,null,2);
  })().catch(e=>{r.error=e.message;r.complete=true;mark('failed');p.stop();});
  return 'Started';
})()
