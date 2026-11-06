import { test, expect, type Page } from "@playwright/test";
import type { Draft, Message } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
test.use({ timezoneId: "Europe/Tallinn" });
const root: Message = { ...mail.rows[0], subject: "Floor heating", preview: "Could you check the thermostat settings?", received_at: "2026-10-28T08:10:00Z", sender: {name: "Maya Chen", email: "maya@example.org"} };
const sent: Message = { ...root, id: "sent-reply", provider_id: "sent", folder_ids: ["Sent Items"], subject: "Re: Floor heating", sender: {name: "Alex Morgan", email: accounts[0].email}, to: [root.sender], received_at: "2026-10-28T08:42:00Z", unread: false, preview: "I’ve checked it. The living room is set to 22 °C." };
async function fixture(page: Page, options: { failure?: boolean; third?: boolean } = {}) {
  const drafts: Draft[] = [], calls: string[] = [];
  let fail = !!options.failure;
  let discovered = false;
  const third: Message = {...root, id:"latest", received_at:"2026-10-28T09:30:00Z", preview:"Thanks, that works."};
  const rows = options.third ? [root,sent,third] : [root,sent];
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname;
    calls.push(path + url.search);
    const respond = (value: unknown, status=200) => route.fulfill({status, contentType:"application/json", body:JSON.stringify(value)});
    if (path === "/api/session") return respond({user_id:"fixture-threading",csrf_token:"fixture"});
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({contentType:"text/event-stream",body:": ready\n\n"});
    if (path === "/api/search" || path.endsWith("/messages")) return respond({...mail, rows:[root, {...root,account_id:"personal",id:"personal-root",subject:"Separate account"}]});
    if (path.endsWith("/conversation")) {
      if (path.includes("/personal/")) return respond({rows:[{...root,account_id:"personal",id:"personal-root",subject:"Separate account"}],continuation:null,complete:true});
      if (url.searchParams.has("snapshot")) return respond({rows:discovered ? rows : [root],continuation:null,complete:discovered});
      if (!url.searchParams.has("continuation")) return respond({rows:[root],continuation:"account-bound-page",complete:false});
      if (fail) { fail=false; return respond({code:"provider_unavailable",message:"temporarily unavailable"},502); }
      discovered = true;
      return respond({rows,continuation:null,complete:true});
    }
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) {
      const id=path.split("/").at(-1);
      const m=rows.find(m=>m.id===id) || {...root,id:"personal-root",account_id:"personal",subject:"Separate account"};
      return respond({...content,message:m,attachments:[],detected_language:"en",message_id:`<${m.id}@example.invalid>`,references:m.id===root.id ? [] : [`<${root.id}@example.invalid>`],reply_to:[], text:m.preview,html:`<p>${m.preview}</p><p>${m.id===sent.id ? "The schedule looks good too. Let me know if you’d like me to adjust anything else." : "The living room felt a little cool this morning. Thank you!"}</p>`});
    }
    if (path.endsWith("/operations")) return respond({...route.request().postDataJSON(),status:"applied",revision:2});
    if (path.endsWith("/recipients")) return respond([]);
    if (path.includes("/drafts/")) { const d=route.request().postDataJSON(); drafts.push(d); return respond({...d,revision:d.revision+1}); }
    if (path.endsWith("/drafts") || path.endsWith("/scheduled") || path.endsWith("/submissions")) return respond([]);
    return respond(null);
  });
  await page.goto("/");
  await page.getByRole("button",{name:/Maya Chen.*Floor heating/}).click();
  return {drafts,calls};
}
for (const [name,width,height,dark] of [
  ["desktop-light",1480,1050,false], ["desktop-dark",1480,1050,true],
  ["mobile-light",390,844,false], ["mobile-dark",390,844,true],
] as const) {
  test(`chronological incoming plus sent reply ${name}`, async ({page}) => {
    await page.setViewportSize({width,height});
    if (dark) await page.addInitScript(()=>localStorage.setItem("dakia-theme","dark"));
    const state=await fixture(page);
    await expect(page.locator(".conversation-count")).toHaveCount(0);
    const cards=page.locator(".conversation-message");
    await expect(cards).toHaveCount(2);
    await expect(cards.nth(0).locator("b")).toHaveText("Maya Chen");
    await expect(cards.nth(1).locator("b")).toHaveText("You");
    await expect(cards.nth(1).locator("iframe")).toBeVisible();
    await expect(cards.nth(1).frameLocator("iframe").locator("body")).toContainText("I’ve checked it");
    // Genuine Chromium screenshot of the running local React app, synthetic API.
    mkdirSync(resolve("../../docs/screenshots/threading"),{recursive:true});
    await page.screenshot({path:resolve(`../../docs/screenshots/threading/${name}.png`),fullPage:true});
    expect(await page.locator(".conversation").evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
    // Development StrictMode may cancel its first mount's request. There is
    // still only one continuation and no polling/refetch amplification.
    expect(state.calls.filter(c=>(c.endsWith("/conversation") || c.includes("/conversation?")) && !c.includes("snapshot=")).length).toBeLessThanOrEqual(3);
    expect(state.calls.filter(c=>c.includes("continuation=")).length).toBe(1);
  });
}
test("pagination failure retains selected content and retry resumes without duplicating, with keyboard full expansion",async({page})=>{
  const state=await fixture(page,{failure:true,third:true});
  await expect(page.getByText("Couldn’t check for more messages",{exact:false})).toBeVisible();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  await page.getByRole("button",{name:"Retry conversation"}).click();
  await expect(page.locator(".conversation-message")).toHaveCount(3);
  const sentCard=page.getByRole("region",{name:"Message from You",exact:true});
  const summary=sentCard.getByRole("button").first();
  await expect(summary).toHaveAttribute("aria-expanded","false");
  await summary.focus(); await page.keyboard.press("Enter");
  await expect(summary).toHaveAttribute("aria-expanded","true");
  await expect(sentCard.frameLocator("iframe").locator("body")).toContainText("schedule looks good");
  await summary.click(); await summary.click();
  await expect(sentCard.frameLocator("iframe").locator("body")).toContainText("schedule looks good");
  expect(state.calls.filter(c=>c.includes("continuation=")).length).toBe(2);
});
test("reply, reply-all and forward use the chosen card, including sent-to-original recipients",async({page})=>{
  const state=await fixture(page);
  const sentCard=page.getByRole("region",{name:"Message from You",exact:true});
  await expect(sentCard.getByRole("button",{name:"Reply",exact:true})).toBeVisible();
  await sentCard.getByRole("button",{name:"Reply",exact:true}).click();
  await page.waitForTimeout(900);
  expect(state.drafts.at(-1)?.to.map(a=>a.email)).toEqual(["maya@example.org"]);
  expect(state.drafts.at(-1)?.in_reply_to).toBe("<sent-reply@example.invalid>");
  expect(state.drafts.at(-1)?.references).toEqual(["<m0@example.invalid>","<sent-reply@example.invalid>"]);
  await page.getByRole("button",{name:"Close",exact:true}).click();
  const original=page.getByRole("region",{name:"Message from Maya Chen",exact:true});
  await original.getByRole("button",{name:"Reply all",exact:true}).click();
  await page.waitForTimeout(900);
  expect(state.drafts.at(-1)?.in_reply_to).toBe("<m0@example.invalid>");
  expect(state.drafts.at(-1)?.to.map(a=>a.email)).toEqual(["maya@example.org"]);
  await page.getByRole("button",{name:"Close",exact:true}).click();
  await sentCard.getByRole("button",{name:"Forward",exact:true}).click();
  await page.waitForTimeout(900);
  expect(state.drafts.at(-1)?.in_reply_to).toBeNull();
  expect(state.drafts.at(-1)?.to).toEqual([]);
});
test("isolated body resizing grows and shrinks, rejects forged frame messages, and preserves sandbox security",async({page})=>{
  await fixture(page);
  const frames=page.locator(".conversation-message iframe");
  await expect(frames).toHaveCount(2);
  const first=frames.first();
  await expect(first).not.toHaveAttribute("sandbox",/allow-same-origin/);
  await expect.poll(async()=> (await first.boundingBox())?.height || 0).toBeLessThan(240);
  const nonce=await first.evaluate(el=>(el as HTMLIFrameElement).srcdoc.match(/nonce="([a-f0-9-]+)"/)![1]);
  const oldHeight=(await first.boundingBox())!.height;
  await page.evaluate(channel=>window.postMessage({kind:"dakia-mail-height",channel,height:99999},"*"),nonce);
  await page.waitForTimeout(80);
  expect((await first.boundingBox())!.height).toBe(oldHeight);
  // The browser's test instrumentation changes only synthetic frame content.
  // This exercises the real opaque-origin ResizeObserver bridge, not a jsdom size mock.
  const body=page.locator(".conversation-message").first().frameLocator("iframe").locator("body");
  await first.evaluate(el => (el as HTMLIFrameElement).contentWindow!.postMessage({kind:"dakia-mail-update", channel:"wrong", source:"<body>Forged update</body>"}, "*"));
  await body.evaluate((el, channel) => window.postMessage({kind:"dakia-mail-update", channel, source:"<body>Forged update</body>"}, "*"), nonce);
  await expect(body).not.toContainText("Forged update");
  await body.evaluate(el=>{ el.innerHTML=Array.from({length:80},(_,i)=>`<p>Synthetic long message paragraph ${i}</p>`).join(""); });
  await expect.poll(async()=> (await first.boundingBox())?.height || 0).toBeGreaterThan(1000);
  await body.evaluate(el=>{ el.innerHTML="<p>Short synthetic message</p>"; });
  await expect.poll(async()=> (await first.boundingBox())?.height || 0).toBeLessThan(120);
  await page.getByRole("region",{name:"Message from You",exact:true}).getByRole("button",{name:"Forward",exact:true}).scrollIntoViewIfNeeded();
  await expect(page.getByRole("region",{name:"Message from You",exact:true}).getByRole("button",{name:"Forward",exact:true})).toBeVisible();
});

test("switching to a separate account never retains previous conversation cards",async({page})=>{
  await fixture(page);
  await expect(page.locator(".conversation-message")).toHaveCount(2);
  await page.getByRole("button",{name:/Separate account/}).click();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  await expect(page.locator(".conversation h2")).toHaveText("Separate account");
  await expect(page.getByRole("region",{name:"Message from You",exact:true})).toHaveCount(0);
});
