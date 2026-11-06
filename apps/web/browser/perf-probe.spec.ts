import { test, expect } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";
import { writeFileSync } from "node:fs";
for (const manual of [false, true]) test(`cache-first loading and body discipline, manual=${manual}`, async ({page}, testInfo) => {
  const root = {...mail.rows[0], unread:false, subject:"Synthetic performance probe"};
  const second = {...root,id:"probe-second",provider_id:"probe-second",received_at:"2026-10-28T12:00:00Z"};
  const third = {...root,id:"probe-third",provider_id:"probe-third",received_at:"2026-10-29T12:00:00Z"};
  const calls: {path:string;ms:number}[]=[];
  let start=0;
  let discovered = 1;
  await page.route("**/api/**", async route => {
    const u=new URL(route.request().url()), p=u.pathname;
    if(start) calls.push({path:p+u.search,ms:Date.now()-start});
    const respond=(v:unknown)=>route.fulfill({contentType:"application/json",body:JSON.stringify(v)});
    if(p==="/api/session") return respond({user_id:"fixture",csrf_token:"fixture"});
    if(p==="/api/accounts") return respond(accounts);
    if(p.endsWith("/events")) return route.fulfill({contentType:"text/event-stream",body:": ready\n\n"});
    if(p==="/api/search" || p.endsWith("/messages")) return respond({...mail,rows:[root]});
    if(p.endsWith("/conversation")) {
      if (u.searchParams.get("snapshot") === "true") return respond({ rows: [root, second, third].slice(0, discovered), continuation: null, complete: discovered === 3 });
      await new Promise(r=>setTimeout(r,800));
      const token=u.searchParams.get("continuation");
      discovered = token === "two" ? 3 : token ? 2 : 1;
      return respond({rows:[root, second, third].slice(0, discovered),continuation:token==="two"?null:token?"two":"one",complete:token==="two"});
    }
    if(p.endsWith("/unsubscribe")) return respond(null);
    if(p.includes("/messages/")) {
      const m=[root,second,third].find(m=>m.id===p.split("/").at(-1))!;
      await new Promise(r=>setTimeout(r,100));
      return respond({...content,message:m,attachments:[],reply_to:[],text:`Body ${m.id}`,html:`<p>Body ${m.id}</p>`});
    }
    if(p.endsWith("/drafts")||p.endsWith("/scheduled")||p.endsWith("/submissions")||p.endsWith("/recipients")) return respond([]);
    return respond(null);
  });
  await page.goto("/");
  start=Date.now();
  await page.getByRole("button",{name:/Synthetic performance probe/}).click();
  await expect(page.locator(".conversation-message").first().frameLocator("iframe").locator("body")).toContainText(`Body ${root.id}`);
  const selectedBodyMs=Date.now()-start;
  if (manual) {
    await page.locator(".conversation-summary").first().click();
    await expect(page.locator(".conversation-message")).toHaveCount(2);
    await page.locator(".conversation-summary").nth(1).click();
  }
  await expect(page.locator(".conversation-message")).toHaveCount(3);
  const discoveryMs=Date.now()-start;
  await expect(page.locator(".conversation-message iframe")).toHaveCount(2);
  expect(calls.some(c=>c.path.endsWith(`/messages/${second.id}`))).toBe(manual);
  await expect(page.locator(".conversation-summary").first()).toHaveAttribute("aria-expanded", manual ? "false" : "true");
  await expect(page.locator(".conversation-summary").nth(1)).toHaveAttribute("aria-expanded", manual ? "true" : "false");
  await expect(page.locator('.conversation-summary[aria-expanded="true"]')).toHaveCount(2);
  const conversationCalls=calls.filter(c=>c.path.includes("/conversation"));
  expect(conversationCalls.filter(c=>c.path.includes("continuation="))).toHaveLength(2);
  expect(selectedBodyMs).toBeLessThan(discoveryMs);
  const output={selectedBodyMs,discoveryMs,expandedCards:await page.locator('.conversation-summary[aria-expanded="true"]').count(),calls};
  writeFileSync(testInfo.outputPath("perf-browser.json"),JSON.stringify(output,null,2));
  await page.screenshot({path:testInfo.outputPath("thread-loading.png"),fullPage:true});
  console.log(JSON.stringify(output));
});
