// Run from repository root after npm run build --workspace @dakia/web.
// Synthetic HTTP fixtures only. No owner mailbox or push subscription is used.
import { chromium } from '@playwright/test';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import assert from 'node:assert/strict';
import { payloadFor } from './service.mjs';
const root = resolve('apps/web/dist');
let online = true, authorized = true, signedIn = true, requests = 0;
const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  if (url.pathname === '/auth/login') {
    signedIn = true; res.writeHead(302, { location:'/' }); res.end(); return;
  }
  if (url.pathname.endsWith('/notification')) {
    requests++;
    if (!online) { req.socket.destroy(); return; }
    res.writeHead(authorized ? 200 : 401, { 'content-type': 'application/json', 'cache-control': 'no-store' });
    res.end(JSON.stringify({ sender: 'Synthetic Sender', subject: 'Browser push acceptance', preview: '<b>Concise preview</b>', account: 'synthetic@example.test' })); return;
  }
  if (url.pathname.startsWith('/api/')) {
    if (!signedIn) { res.writeHead(401, { 'content-type':'application/json' }); res.end(JSON.stringify({code:'login_required'})); return; }
    res.writeHead(200, { 'content-type': 'application/json' });
    const message = { id:'m',account_id:'a',thread_id:'t',provider_id:'p',folder_ids:['INBOX'],sender:{name:'Synthetic Sender',email:'sender@example.test'},to:[],subject:'Browser push acceptance',preview:'Concise preview',received_at:'2026-10-30T00:00:00Z',unread:false,starred:false,category:null,category_status:'pending',revision:1 };
    let data = [];
    if (url.pathname === '/api/session') data = { user_id:'synthetic',csrf_token:'synthetic' };
    if (url.pathname === '/api/accounts') data = [{id:'a',email:'synthetic@example.test',display_name:'Synthetic',provider:'imap',state:'ready',revision:1}];
    if (url.pathname === '/api/search') data = {rows:[message],coverage:[],classification:{classified:0,total:1},counts_exact:true,pending:false,continuation:null};
    if (url.pathname.endsWith('/messages/m')) data = { message,text:'Synthetic browser message. No real mail used.',html:'',attachments:[],cc:[],reply_to:[],message_id:null,references:[] };
    if (url.pathname.endsWith('/conversation')) data = {rows:[message],continuation:null,complete:true};
    if (url.pathname.endsWith('/sync-active')) data = false;
    res.end(JSON.stringify(data)); return;
  }
  const path = resolve(root, '.' + url.pathname);
  try {
    const bytes = await readFile(path === root || !extname(path) ? root + '/index.html' : path);
    res.writeHead(200, { 'content-type': ({'.js':'text/javascript','.css':'text/css','.png':'image/png','.webmanifest':'application/manifest+json'})[extname(path)] || 'text/html', 'cache-control':'no-store' }); res.end(bytes);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(r => server.listen(4193, '127.0.0.1', r));
const origin = 'http://127.0.0.1:4193';
const context = await chromium.launchPersistentContext(resolve('verification/browser-profile'), { executablePath:process.env.CHROME_PATH || '/usr/bin/chromium',headless:true,args:['--no-sandbox'],permissions:['notifications'],viewport:{width:1280,height:900} });
try {
  const page = context.pages()[0];
  const cdp = await context.newCDPSession(page);
  const registrations = [];
  cdp.on('ServiceWorker.workerRegistrationUpdated', e => registrations.push(...e.registrations));
  await cdp.send('ServiceWorker.enable');
  await page.goto(origin);
  await page.evaluate(async () => { await navigator.serviceWorker.register('/sw.js'); await navigator.serviceWorker.ready; });
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null);
  await page.evaluate(async () => { for (const n of await (await navigator.serviceWorker.ready).getNotifications()) n.close(); });
  const registration = registrations.findLast(r => r.scopeURL === origin + '/');
  assert.ok(registration);
  async function notifications() { return page.evaluate(async () => (await (await navigator.serviceWorker.ready).getNotifications()).map(n => ({title:n.title,body:n.body,tag:n.tag,data:n.data}))); }
  async function push(job) {
    await cdp.send('ServiceWorker.deliverPushMessage', { origin, registrationId:registration.registrationId,data:payloadFor(job) });
  }
  const job = {id:'a:m:s',account_id:'a',message_id:'m'};
  await push(job);
  for (let i=0;i<50 && !(await notifications()).length;i++) await page.waitForTimeout(100);
  let shown = await notifications();
  assert.equal(shown[0].title, 'Synthetic Sender · Browser push acceptance');
  assert.equal(shown[0].body, 'synthetic@example.test\nConcise preview');
  await push(job); await page.waitForTimeout(500);
  assert.equal((await notifications()).length,1);
  await push({...job,id:'b:m:s',account_id:'b'}); await page.waitForTimeout(500);
  assert.equal((await notifications()).length,2);
  authorized=false;
  await push({...job,id:'a:missing:s',message_id:'missing'}); await page.waitForTimeout(500);
  assert.equal((await notifications()).find(n=>n.tag==='dakia:a:missing:s').title,'New mail');
  online=false;
  await push({...job,id:'a:offline:s',message_id:'offline'}); await page.waitForTimeout(1000);
  assert.equal((await notifications()).find(n=>n.tag==='dakia:a:offline:s').title,'New mail');
  const worker = context.serviceWorkers().find(w=>w.url()===origin+'/sw.js');
  assert.ok(worker);
  await worker.evaluate(async () => { const notification=(await self.registration.getNotifications()).find(n=>n.tag==='dakia:a:m:s'); self.dispatchEvent(new NotificationEvent('notificationclick',{notification})); });
  await page.waitForURL(origin+'/?account=a&message=m');
  await page.frameLocator('iframe[title="Message body"]').getByText('Synthetic browser message. No real mail used.',{exact:true}).waitFor();
  await mkdir('verification/screenshots',{recursive:true});
  await page.screenshot({path:'verification/screenshots/chromium-notification-click.png',fullPage:true});
  assert.deepEqual(await page.evaluate(()=>caches.keys()),[]);
  const exactMessageClick = page.url();
  signedIn = false;
  await page.goto(origin+'/?account=a&message=m');
  await page.getByRole('link',{name:'Sign in',exact:true}).click();
  await page.waitForURL(origin+'/');
  await page.frameLocator('iframe[title="Message body"]').getByText('Synthetic browser message. No real mail used.',{exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>sessionStorage.getItem('dakia-notification-target')),null);
  const visibleNotifications = await notifications();
  const control = await context.newPage();
  const background = await context.newCDPSession(control);
  await background.send('ServiceWorker.enable');
  await page.close(); // Only an about:blank control page remains, no app window.
  online = true; authorized = true;
  await background.send('ServiceWorker.deliverPushMessage', { origin, registrationId:registration.registrationId,data:payloadFor({...job,id:'a:background:s',message_id:'background'}) });
  let backgroundNotification;
  for (let i=0;i<50;i++) {
    backgroundNotification = await worker.evaluate(async () => (await self.registration.getNotifications()).find(n=>n.tag==='dakia:a:background:s')?.title);
    if (backgroundNotification) break;
    await control.waitForTimeout(100);
  }
  assert.equal(backgroundNotification,'Synthetic Sender · Browser push acceptance');
  const report={browser:await context.browser()?.version() || 'Chromium persistent',realServiceWorker:true,cdpPushEvents:true,backgroundNoAppWindow:true,syntheticSignInPreservesTarget:true,osPopupVerified:false,externalPushTransport:false,syntheticOnly:true,initialNotification:shown[0],visibleNotifications,requests,exactMessageClick,noCache:true};
  await writeFile('verification/browser-push.json',JSON.stringify(report,null,2));
  console.log(JSON.stringify(report,null,2));
} finally { await context.close(); server.close(); }
