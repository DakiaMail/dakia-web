import { test, expect, type Page } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";
async function setup(page: Page) {
  await page.addInitScript(() => {
    const streams: {url:string; closed:boolean; send:(cursor:number, kind:string)=>void}[] = [];
    class Source {
      closed = false;
      onmessage?: (event: {data:string}) => void;
      onerror?: () => void;
      constructor(public url: string) { streams.push(this); }
      close() {this.closed=true;}
      addEventListener() {}
      send(cursor: number, kind: string) {if(!this.closed)this.onmessage?.({data:JSON.stringify({account_id:this.url.split('/')[3],cursor,kind,target_id:'m0',revision:1})});}
    }
    Object.assign(window,{EventSource:Source, testStreams:streams});
  });
  await page.route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    if(path === "/api/session")return route.fulfill({json:{user_id:"fixture",csrf_token:"fixture"}});
    if(path === "/api/accounts")return route.fulfill({json:accounts});
    if(path === "/api/search" || path.endsWith("/messages"))return route.fulfill({json:mail});
    return route.fulfill({json:[]});
  });
}
async function burst(page: Page, id: string, kind: string) {
  await page.evaluate(({id,kind}) => {
    const streams = (window as unknown as {testStreams:{url:string;send:(cursor:number,kind:string)=>void}[]}).testStreams;
    const source=[...streams].reverse().find(s=>s.url.includes(`/${id}/`));
    for(let i=1;i<=500;i++)source?.send(i,kind);
  },{id,kind});
}
test("idle, event bursts and account switching have bounded request counts", async ({page}) => {
  await setup(page);
  const counts={search:0,work:0,personal:0};
  page.on("request", r=>{const u=new URL(r.url()); if(u.pathname === '/api/search') { const ids=u.searchParams.get('accounts'); if(ids==='personal') counts.personal++; else if(ids==='work') counts.work++; else counts.search++; }});
  await page.goto('/');
  await expect(page.getByText('A few thoughts on the launch',{exact:true})).toBeVisible();
  await page.waitForTimeout(2500);
  expect(counts.search).toBe(1);
  await burst(page,'work','message_changed');
  await page.waitForTimeout(2400);
  expect(counts.search).toBe(2); // 500 changes, one refetch
  await page.getByRole('navigation',{name:'Accounts'}).getByRole('button',{name:/Personal/}).click();
  await expect.poll(()=>counts.personal).toBe(1);
  await burst(page,'work','message_changed'); // duplicate cursors, wrong selected scope
  await page.waitForTimeout(2300);
  expect(counts.personal).toBe(1);
  await page.getByRole('button',{name:'All accounts',exact:true}).click();
  await page.waitForTimeout(500);
  expect(counts.search).toBe(2); // cached navigation
});
test("navigation aborts the obsolete fetch instead of leaking pending searches", async ({page}) => {
  await setup(page);
  const aborted:string[]=[];
  page.on('requestfailed',r=>{if(new URL(r.url()).pathname==='/api/search')aborted.push('search');});
  let release!:()=>void;
  const held=new Promise<void>(r=>{release=r;});
  let started=false;
  await page.route('**/api/search?**',async route=>{if(new URL(route.request().url()).searchParams.get('accounts')==='personal')return route.fulfill({json:mail});started=true;await held;await route.fulfill({json:mail}).catch(()=>{});});
  await page.goto('/');
  await expect.poll(()=>started).toBe(true);
  await page.getByRole('navigation',{name:'Accounts'}).getByRole('button',{name:/Personal/}).click();
  await expect.poll(()=>aborted.length).toBe(1);
  release();
  await expect(page.getByText('A few thoughts on the launch',{exact:true})).toBeVisible();
});
