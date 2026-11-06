import { test, expect, type Page } from "@playwright/test";
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { accounts, content, page as mail } from "./fixtures";

const png = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAFAAAAAgCAIAAAAKUWg7AAAAQklEQVR4nO3PAQkAIBDAQCMawKxmM4awP1iA3Tr7jmp9PwAGBgYGBgYeE3A94HrA9YDrAdcDrgdcD7gecD3gesD1HgWJJNNntUVKAAAAAElFTkSuQmCC';
const receipt = `<html lang="en"><head><style>
body{margin:0;padding:16px;font:16px/1.5 Arial;background:#fff;color:#111}
.columns{font-size:0}.picture,.details{display:inline-block;vertical-align:top;font-size:16px}.picture{width:30%}.details{width:70%}
.product{width:100%;max-width:64px;max-height:64px;height:auto}.tracker{display:flex;gap:0}.step{width:25%;text-align:center;white-space:nowrap}.connector{height:4px;background:#1299ab;margin:12px 0}.desktop{display:none}
@media(min-width:600px){.desktop{display:block}.mobile{display:none}.picture{width:144px}.details{width:calc(100% - 144px)}}
.hero{height:40px;background:url(https://images.example.org/banner.png) center/cover #eee}.price sup{font-size:10px}
</style></head><body class="receipt"><h1>Thanks for your order</h1><div class="desktop">Desktop receipt</div><div class="mobile">Mobile receipt</div><div class="tracker">${['Ordered','Dispatched','On the way','Delivered'].map(s=>`<div class="step"><div class="connector"></div>${s}</div>`).join('')}</div><div class="hero"></div><div class="columns"><div class="picture"><img class="product" src="https://images.example.org/product.png" alt="Product"></div><div class="details"><a href="https://example.org/order" style="text-decoration:none">Travel mug</a><div class="price">€11<sup>99</sup></div></div></div><p class="end">Receipt complete</p></body></html>`;
const hostile = String.raw`<style>@import 'https://tracker.example/import';@font-face{font-family:evil;src:url(https://tracker.example/font)}.bad{background:u\72l(https://tracker.example/css);cursor:url(https://tracker.example/cursor),auto;behavior:url(https://tracker.example/htc);width:expression(alert(1))}.set{background:image-set('https://tracker.example/set' 1x)}.safe{display:grid;grid-template-columns:1fr 2fr}</style><meta http-equiv="refresh" content="0;url=https://tracker.example/redirect"><link rel="stylesheet" href="https://tracker.example/style"><base href="https://tracker.example"><script>parent.document.body.textContent='compromised';fetch('https://tracker.example/script')</script><form action="https://tracker.example/form"><input autofocus onfocus="alert(1)"></form><iframe src="https://tracker.example/frame"></iframe><svg onload="alert(1)"><image href="https://tracker.example/svg"></image></svg><math><mtext><table><mglyph><style><!--</style><img title="--><img src=x onerror=alert(1)>"></mglyph></table></mtext></math><div class="safe" onclick="alert(1)">Safe content</div><div class="bad">Blocked resources</div><div class="set">Blocked set</div><img src="https://tracker.example/pixel" hidden width="1" height="1"><a href="javascript:alert(1)">Unsafe</a><a href="data:text/html,broken">Data</a><a href="https://example.org/safe">Safe link</a>`;

async function openMail(page: Page, html: string, images: Record<string,string> = {}, block = false) {
  const external: string[] = [], imageRequests: string[] = [], writes: string[] = [];
  await page.addInitScript(block => {
    localStorage.setItem('dakia:layout-fixture:always-load-images', 'false');
    localStorage.setItem('dakia:layout-fixture:block-hidden-images', String(block));
  }, block);
  const root = { ...content.message, unread: false };
  await page.route('**/*', route => {
    const url = new URL(route.request().url()), path = url.pathname;
    if (url.origin !== 'http://127.0.0.1:4173') { external.push(url.origin); return route.abort(); }
    if (!path.startsWith('/api/')) return route.continue();
    const respond = (json: unknown) => route.fulfill({ json });
    if (path.endsWith('/image')) {
      const source = route.request().postDataJSON().url; imageRequests.push(source);
      return respond(images[source] || png);
    }
    if (route.request().method() !== 'GET') writes.push(path);
    if (path === '/api/session') return respond({ user_id: 'layout-fixture', csrf_token: 'fixture' });
    if (path === '/api/accounts') return respond(accounts);
    if (path.endsWith('/events')) return route.fulfill({ contentType:'text/event-stream', body:': ready\n\n' });
    if (path === '/api/search' || path.endsWith('/messages')) return respond({ ...mail, rows:[root] });
    if (path.endsWith('/conversation')) return respond({ rows:[root], complete:true, continuation:null });
    if (path.endsWith('/messages/m0')) return respond({ ...content, message:root, html, text:'Plain message', detected_language:'en' });
    return respond(null);
  });
  await page.goto('/'); await page.locator('.message-open').click();
  const frame = page.frameLocator('iframe[title="Message body"]');
  await expect(frame.locator('body')).not.toBeEmpty();
  return { frame, external, imageRequests, writes };
}
for (const width of [320, 390, 600, 900, 1480, 1920]) test(`responsive receipt preserves geometry and CSS resources ${width}`, async ({page}) => {
  await page.setViewportSize({width, height:980});
  const {frame,external,imageRequests,writes} = await openMail(page, receipt);
  await expect(frame.locator('.hero')).toHaveCSS('background-image', 'none');
  expect(imageRequests).toEqual([]);
  await page.getByRole('button', {name:'Load images',exact:true}).click();
  await expect(frame.locator('.product')).toHaveJSProperty('naturalWidth',80);
  await expect(frame.locator('.hero')).toHaveCSS('background-image', `url("${png}")`);
  const geometry = await frame.locator('body').evaluate(body => {
    const rect = (selector:string) => {const r=body.querySelector(selector)!.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};};
    return { width:innerWidth, overflow:body.scrollWidth-innerWidth, picture:rect('.picture'), details:rect('.details'), product:rect('.product'),sup:rect('sup'), price:rect('.price'), line:rect('.connector') };
  });
  expect(geometry.product.width).toBeLessThanOrEqual(64);
  expect(geometry.details.x).toBeGreaterThan(geometry.picture.x);
  expect(geometry.details.y).toBeCloseTo(geometry.picture.y);
  expect(geometry.line.height).toBe(4);
  expect(geometry.sup.height).toBeLessThan(geometry.price.height);
  await expect(frame.locator(geometry.width >=600 ? '.desktop' : '.mobile')).toBeVisible();
  await expect(frame.locator(geometry.width >=600 ? '.mobile' : '.desktop')).toBeHidden();
  expect(geometry.overflow).toBeLessThanOrEqual(1);
  // Stable document height, complete final content and no history entry per image.
  const iframe=page.locator('iframe[title="Message body"]');
  const height=(await iframe.boundingBox())!.height;
  await expect.poll(async()=>(await iframe.boundingBox())!.height).toBe(height);
  await frame.locator('.end').scrollIntoViewIfNeeded(); await expect(frame.locator('.end')).toBeInViewport();
  expect(imageRequests.sort()).toEqual(['https://images.example.org/banner.png','https://images.example.org/product.png']);
  expect(external).toEqual([]); expect(writes).toEqual([]);
  mkdirSync('artifacts/email-layouts',{recursive:true});
  await page.screenshot({path:`artifacts/email-layouts/receipt-${width}.png`});
});

for(const name of ['mailchimp-body-image','mailchimp-two-column','colspan-newsletter']) for(const width of [390,1480]) test(`original public template ${name} ${width}`,async({page})=>{
  await page.setViewportSize({width,height:980});
  const html=readFileSync(new URL(`./fixtures/${name}.html`,import.meta.url),'utf8');
  const {frame,external,writes}=await openMail(page,html);
  await expect(frame.locator('table').first()).toHaveCSS('display','table');
  const paragraph=frame.locator('p,td').filter({hasText:/\S.{30}/}).last();
  await paragraph.scrollIntoViewIfNeeded();await expect(paragraph).toBeVisible();
  // Authored fixed widths may scroll inside the isolated document; they must
  // never overflow the app or become anonymous block tables.
  expect(await page.evaluate(()=>document.documentElement.scrollWidth-innerWidth)).toBeLessThanOrEqual(1);
  expect(external).toEqual([]);expect(writes).toEqual([]);
  mkdirSync('artifacts/email-layouts',{recursive:true});
  await page.screenshot({path:`artifacts/email-layouts/${name}-${width}.png`});
});

for(const width of [390,1480]) test(`hostile HTML CSS and frame boundary ${width}`,async({page})=>{
  await page.setViewportSize({width,height:980});
  let dialogs=0;page.on('dialog',async dialog=>{dialogs++;await dialog.dismiss();});
  const {frame,external,imageRequests,writes}=await openMail(page,hostile,{},true);
  await expect(frame.locator('.safe')).toHaveCSS('display','grid');
  expect(await frame.locator('.safe').evaluate(()=>{try{void parent.document;return false;}catch{return true;}})).toBe(true);
  await expect(frame.locator('script:not([nonce]),form,input,iframe,svg,math,link,base,meta[http-equiv="refresh"]')).toHaveCount(0);
  await expect(frame.locator('a:not([href])')).toHaveCount(2);
  await expect(frame.locator('.set')).toHaveCSS('background-image','none');
  await expect(page.locator('iframe[title="Message body"]')).toHaveAttribute('sandbox','allow-scripts allow-popups allow-popups-to-escape-sandbox');
  // Even forged resize reports cannot change the frame from the parent origin.
  const iframe=page.locator('iframe[title="Message body"]'); const height=(await iframe.boundingBox())!.height;
  await page.evaluate(()=>window.postMessage({kind:'dakia-mail-height',channel:'forged',height:99999},'*'));
  expect((await iframe.boundingBox())!.height).toBe(height);
  await expect(frame.getByRole('link',{name:'Safe link'})).toHaveAttribute('rel','noopener noreferrer');
  expect(imageRequests).toEqual([]);expect(external).toEqual([]);expect(writes).toEqual([]);expect(dialogs).toBe(0);
});

test('fixed-width mail fits without rewriting tables and can show original size',async({page})=>{
  await page.setViewportSize({width:390,height:980});
  const html='<table width="700" style="width:700px"><tr><td style="padding:16px"><a href="https://example.org/fixed" style="display:block;width:660px"><img src="https://images.example.org/banner.png" style="width:660px;height:264px" alt="Banner"></a><p>Fixed-width newsletter</p></td></tr></table>';
  const {frame,external,writes}=await openMail(page,html);
  await page.getByRole('button',{name:'Load images',exact:true}).click();
  await expect(frame.getByAltText('Banner')).toHaveJSProperty('naturalWidth',80);
  await expect(page.getByRole('button',{name:'Original size',exact:true})).toBeVisible();
  await expect(frame.locator('table')).toHaveCSS('width','700px');
  const iframe=page.locator('iframe[title="Message body"]');
  await expect.poll(()=>iframe.evaluate(node=>getComputedStyle(node).transform)).not.toBe('none');
  const visualWidth=async()=>{
    const scale=await iframe.evaluate(node=>node.getBoundingClientRect().width/(node as HTMLIFrameElement).clientWidth);
    return (await frame.getByAltText('Banner').evaluate(node=>node.getBoundingClientRect().width))*scale;
  };
  const fittedWidth=await visualWidth();
  expect(fittedWidth).toBeLessThan(390);
  await page.context().route('https://example.org/fixed',route=>route.fulfill({contentType:'text/html',body:'Intercepted fixture link'}));
  const popupPromise=page.context().waitForEvent('page');
  await frame.getByRole('link').click();
  const popup=await popupPromise;await popup.waitForLoadState();
  expect(popup.url()).toBe('https://example.org/fixed');
  expect(await popup.evaluate(()=>window.opener===null)).toBe(true);await popup.close();
  await page.getByRole('button',{name:'Original size',exact:true}).click();
  const original=(await frame.getByAltText('Banner').boundingBox())!;
  expect(original.width).toBe(660);expect(original.height).toBe(264);
  await expect(page.getByRole('button',{name:'Fit to width',exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Fit to width',exact:true}).click();
  await expect.poll(visualWidth).toBeCloseTo(fittedWidth);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth-innerWidth)).toBeLessThanOrEqual(1);
  await page.setViewportSize({width:1920,height:980});
  await expect(page.getByRole('button',{name:'Original size',exact:true})).toHaveCount(0);
  await page.setViewportSize({width:320,height:980});
  await expect(page.getByRole('button',{name:'Original size',exact:true})).toBeVisible();
  expect(external).toEqual([]);expect(writes).toEqual([]);
});

test('viewport-sized message height settles without adding browser history',async({page})=>{
  const {frame,external}=await openMail(page,'<html style="height:100%"><body style="height:100%;margin:0;padding:0"><p>Viewport message</p></body></html>');
  await expect(frame.getByText('Viewport message')).toBeVisible();
  await expect(async()=>{
    const samples=await page.locator('iframe[title="Message body"]').evaluate(async node=>{
      const samples:number[]=[];
      for(let i=0;i<12;i++){await new Promise(requestAnimationFrame);samples.push(node.getBoundingClientRect().height);}
      return samples;
    });
    expect(Math.max(...samples)-Math.min(...samples)).toBeLessThanOrEqual(1);
  }).toPass({timeout:5000});
  expect(external).toEqual([]);
});

// Owner message stays outside source control. Run with a private directory that
// contains amazon.json and amazon-images.json; no provider or link is contacted.
const privateDir=process.env.DAKIA_PRIVATE_AMAZON;
if(privateDir) for(const width of [320,390,600,900,1480,1920]) test(`exact private Amazon receipt ${width}`,async({page})=>{
  await page.setViewportSize({width,height:1100});
  const {html}=JSON.parse(readFileSync(resolve(privateDir,'amazon.json'),'utf8'));
  const images:Record<string,string>=JSON.parse(readFileSync(resolve(privateDir,'amazon-images.json'),'utf8'));
  const {frame,external,imageRequests,writes}=await openMail(page,html,images,true);
  await page.getByRole('button',{name:'Load images',exact:true}).click();
  const products=frame.locator('img[src]').filter({visible:true});
  await expect.poll(()=>frame.locator('img[style*="max-width:64px"]').evaluateAll(nodes=>nodes.every(n=>(n as HTMLImageElement).naturalWidth>0))).toBe(true);
  await expect.poll(()=>imageRequests.length).toBe(7);
  await expect(frame.locator('img')).toHaveCount(9);
  const geometry=await frame.locator('body').evaluate(body=>{
    const product=[...body.querySelectorAll<HTMLImageElement>('img')].filter(n=>n.style.maxWidth==='64px');
    return {width:innerWidth,overflow:body.scrollWidth-innerWidth,products:product.map(n=>{const r=n.getBoundingClientRect();return {width:r.width,height:r.height,decoded:n.naturalWidth>0};}),supCount:body.querySelectorAll('sup').length,visibleTables:[...body.querySelectorAll('table')].filter(n=>getComputedStyle(n).display==='table').length,connectors:[...body.querySelectorAll<HTMLElement>('[style]')].filter(n=>n.style.height==='4px').map(n=>({width:n.getBoundingClientRect().width,height:n.getBoundingClientRect().height}))};
  });
  expect(geometry.products).toHaveLength(4);
  expect(geometry.products.every(n=>n.decoded && n.width<=(geometry.width>=600?122.1:64.1) && n.height<=(geometry.width>=600?122.1:64.1) && n.width>0)).toBe(true);
  expect(geometry.supCount).toBeGreaterThanOrEqual(4);
  expect(geometry.visibleTables).toBeGreaterThan(5);
  expect(geometry.overflow).toBeLessThanOrEqual(1);
  await expect(frame.getByText('Dispatched',{exact:true})).toBeVisible();
  expect(imageRequests.every(url=>url.startsWith('https://m.media-amazon.com/images/')&&!url.includes('?'))).toBe(true);
  expect(external).toEqual([]);expect(writes).toEqual([]);
  const output=resolve(privateDir,'..','screenshots');mkdirSync(output,{recursive:true,mode:0o700});
  await products.first().scrollIntoViewIfNeeded();
  await page.screenshot({path:`${output}/amazon-app-${width}.png`});
  const isolated = await frame.locator('html').evaluate(node=>{
    const clone=node.cloneNode(true) as HTMLElement;
    clone.querySelectorAll('script').forEach(n=>n.remove());
    return '<!doctype html>'+clone.outerHTML;
  });
  const capture=await page.context().newPage();
  await capture.route('**/*',route=>route.abort());
  await capture.setViewportSize({width:geometry.width,height:1100});
  await capture.setContent(isolated);
  await capture.locator('img[style*="max-width:64px"]').last().waitFor();
  await expect.poll(()=>capture.locator('img[style*="max-width:64px"]').evaluateAll(nodes=>nodes.every(n=>(n as HTMLImageElement).naturalWidth>0))).toBe(true);
  await capture.screenshot({path:`${output}/amazon-${width}.png`,fullPage:true});
  const reference = await capture.evaluate(({html,images})=>{
    const doc=new DOMParser().parseFromString(html,'text/html');
    doc.querySelectorAll('script,link,base,meta').forEach(n=>n.remove());
    for(const node of doc.querySelectorAll('*')) for(const attr of [...node.attributes]) if(/^on/i.test(attr.name))node.removeAttribute(attr.name);
    for(const img of doc.querySelectorAll('img')){const data=images[img.src];if(data)img.src=data;else img.remove();}
    const csp=doc.createElement('meta');csp.httpEquiv='Content-Security-Policy';csp.content="default-src 'none';script-src 'none';style-src 'unsafe-inline';img-src data:;font-src 'none';base-uri 'none';form-action 'none'";doc.head.prepend(csp);
    return '<!doctype html>'+doc.documentElement.outerHTML;
  },{html,images});
  await capture.setContent(reference);
  await expect.poll(()=>capture.locator('img[style*="max-width:64px"]').evaluateAll(nodes=>nodes.every(n=>(n as HTMLImageElement).naturalWidth>0))).toBe(true);
  const original=await capture.locator('img[style*="max-width:64px"]').evaluateAll(nodes=>nodes.map(n=>{const r=n.getBoundingClientRect();return {width:r.width,height:r.height};}));
  expect(geometry.products.map(({width,height})=>({width,height}))).toEqual(original);
  await capture.screenshot({path:`${output}/amazon-original-${width}.png`,fullPage:true});
  await capture.close();
  writeFileSync(`${output}/amazon-${width}.json`,JSON.stringify(geometry,null,2),{mode:0o600});
});

for (const sizing of ['min-height:100vh', 'height:100vh', 'min-height:100%', 'min-height:110vh;box-sizing:content-box']) test(`viewport body padding has bounded stable height ${sizing}`, async ({page}) => {
  const { frame } = await openMail(page, `<body style="${sizing};padding:16px"><p>Viewport padding</p><div id="late"></div><p id="end">Message end</p></body>`);
  const iframe=page.locator('iframe[title="Message body"]');
  await expect(async()=>{
    const samples=await iframe.evaluate(async node=>{
      const values:number[]=[];
      for(let i=0;i<20;i++){await new Promise(requestAnimationFrame);values.push(node.getBoundingClientRect().height);}
      return values;
    });
    expect(Math.max(...samples)).toBeLessThan(2000);
    expect(Math.max(...samples)-Math.min(...samples)).toBeLessThanOrEqual(1);
  }).toPass({timeout:3000});
  // Content arriving after stabilization must still expand the reader.
  const before = await iframe.evaluate(node => (node as HTMLIFrameElement).clientHeight);
  await frame.locator('#late').evaluate((node, source) => {
    const image = document.createElement('img');
    image.style.cssText = 'display:block;width:1px;height:1200px';
    image.src = source;
    node.append(image);
  }, png);
  await expect.poll(() => iframe.evaluate(node => (node as HTMLIFrameElement).clientHeight)).toBeGreaterThan(before + 1000);
  await frame.locator('#end').scrollIntoViewIfNeeded();
  await expect(frame.locator('#end')).toBeInViewport();
});
