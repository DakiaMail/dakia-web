import { test, expect, type Page } from "@playwright/test";
import type { Message } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

const root = { ...mail.rows[0], subject: "Refresh fixture" };
const reply = { ...root, id: "unseen-reply", subject: "Re: Refresh fixture", sender: {name:"New reply",email:"reply@example.invalid"} };
async function fixture(page: Page, slow = false) {
  await page.addInitScript(() => {
    const sources: any[] = [];
    (window as any).emitMail = (account: string, cursor: number, kind = "message_changed", target = "not-visible") => {
      for (const source of sources.filter(s => s.url.includes(`/accounts/${account}/`))) {
        source.onmessage?.({data:JSON.stringify({account_id:account,cursor,kind,target_id:target,revision:1})});
      }
    };
    (window as any).EventSource = class {
      onmessage: any;
      constructor(public url: string) { sources.push(this); }
      addEventListener() {}
      close() { sources.splice(sources.indexOf(this),1); }
    };
  });
  const state = { rows: [root] as Message[], snapshots: 0, discovery: 0, continuations: 0, release: () => {}, fail: false };
  const gate = new Promise<void>(resolve => { state.release = resolve; });
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (value: unknown, status=200) => route.fulfill({status,contentType:"application/json",body:JSON.stringify(value)});
    if (path === "/api/session") return respond({user_id:"fixture-refresh",csrf_token:"fixture"});
    if (path === "/api/accounts") return respond(accounts);
    if (path === "/api/search" || path.endsWith("/messages")) return respond({...mail,rows:[root]});
    if (path.endsWith("/conversation")) {
      if (url.searchParams.has("snapshot")) { state.snapshots++; return respond({rows:state.rows,complete:!slow,continuation:null}); }
      if (url.searchParams.has("continuation")) {
        state.continuations++;
        if (slow) await gate;
        return respond({rows:[root],complete:true,continuation:null});
      }
      state.discovery++;
      if (state.fail) return respond({code:"provider_unavailable"},502);
      return respond({rows:[root],complete:!slow,continuation:slow ? "shared" : null});
    }
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) return respond({...content,message:state.rows.find(m=>path.endsWith(`/${m.id}`)) || root});
    if (path.endsWith("/events")) return route.fulfill({contentType:"text/event-stream",body:": fixture\n\n"});
    if (path.endsWith("/operations")) return respond({...route.request().postDataJSON(),status:"applied",revision:2});
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("button",{name:/Refresh fixture/}).click();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  return state;
}
async function emit(page: Page, account: string, first: number, count=1, kind="message_changed") {
  await page.evaluate(({account,first,count,kind}) => {
    for(let i=0;i<count;i++) (window as any).emitMail(account,first+i,kind);
  },{account,first,count,kind});
}

test("500 unrelated events refresh once locally, unknown new replies appear, other accounts stay isolated",async({page})=>{
  const state = await fixture(page);
  await page.waitForTimeout(200);
  const baseline = {snapshots:state.snapshots,discovery:state.discovery};
  await emit(page,"work",1,500);
  await expect.poll(()=>state.snapshots).toBe(baseline.snapshots+1);
  await page.waitForTimeout(100);
  expect(state.discovery).toBe(baseline.discovery);
  expect(state.continuations).toBe(0);
  await expect(page.getByText("Finding messages…",{exact:true})).toHaveCount(0);
  state.rows = [root,reply];
  await emit(page,"work",501,1,"future_unknown_event");
  await expect(page.locator(".conversation-message")).toHaveCount(2);
  expect(state.discovery).toBe(baseline.discovery);
  const beforeOther = state.snapshots;
  await emit(page,"personal",1,30);
  await page.waitForTimeout(2200);
  expect(state.snapshots).toBe(beforeOther);
  // Metadata snapshots can remove members too, without stale-card resurrection.
  state.rows = [root];
  await emit(page,"work",502,1,"reset");
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  console.log(JSON.stringify({case:"500-event-burst",snapshot_requests:1,discovery_requests:0,unknown_reply_discovered:true,other_account_requests:0}));
});

test("events during slow verification keep the count stable and final local catch-up wins",async({page})=>{
  const state = await fixture(page,true);
  await expect.poll(()=>state.continuations).toBe(1);
  state.rows = [root,reply];
  await emit(page,"work",1,100);
  await expect(page.locator(".conversation-message")).toHaveCount(2);
  await page.locator(".conversation-messages").evaluate(el => {
    (window as any).countChanges = [];
    new MutationObserver(()=> (window as any).countChanges.push(el.textContent)).observe(el,{subtree:true,characterData:true,childList:true});
  });
  const discovery = state.discovery, snapshots = state.snapshots;
  state.release();
  await expect.poll(()=>state.snapshots).toBeGreaterThan(snapshots);
  await expect(page.locator(".conversation-message")).toHaveCount(2);
  expect(state.discovery).toBe(discovery);
  expect(state.continuations).toBe(1);
  expect(await page.evaluate(()=> (window as any).countChanges)).not.toContain("1 message loaded");
  await expect(page.locator(".conversation-count")).toHaveCount(0);
  await expect(page.getByText("Showing available messages.",{exact:false})).toHaveCount(0);
  await expect(page.getByText("Finding messages…",{exact:true})).toHaveCount(0);
});

test("reading does not poll the provider and explicit refresh preserves known cards on failure",async({page})=>{
  await page.clock.install();
  const state = await fixture(page);
  await page.waitForTimeout(100);
  const before = state.discovery;
  await page.clock.fastForward(121000);
  await expect.poll(()=>state.discovery).toBe(before);
  state.fail = true;
  await page.getByRole("button",{name:"Check for more messages",exact:true}).click();
  await expect(page.getByText("Couldn’t check for more messages",{exact:false})).toBeVisible();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  state.fail = false;
  await page.getByRole("button",{name:"Retry conversation"}).click();
  await expect(page.getByText("Couldn’t check for more messages",{exact:false})).toHaveCount(0);
  expect(state.discovery).toBe(before+2);
});
