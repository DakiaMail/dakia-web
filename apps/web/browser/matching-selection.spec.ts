import { test, expect, type Page } from "@playwright/test";
import { accounts, content, messages, page as mail } from "./fixtures";
import type { Message, Operation } from "@dakia/api-types";
async function fixture(page: Page, incomplete = false, slow = false) {
  const rows: Message[] = Array.from({ length: 53 }, (_, i) => ({ ...messages[0], id: `match-${i}`, thread_id: `match-${i}`, subject: `Match ${i}`, unread: false,
    classification: { kind: "automated", status: "ready", types: ["security_alert"], details: [], attention: [], attention_override: null, source: "automatic", policy_revision: "personal-first-1" } }));
  const operations: Operation[] = []; let pages = 0;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), p = url.pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if(p === "/api/session") return respond({ user_id: "matching-selection-fixture", csrf_token: "fixture" });
    if(p === "/api/accounts") return respond([accounts[0]]);
    if(p.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if(p.endsWith("/operations")) { const op = { ...route.request().postDataJSON(), status: "applied", revision: 2 }; operations.push(op); return respond(op); }
    if(p === "/api/search" || p.endsWith("/messages")) {
      const empty = url.searchParams.has("attention") || url.searchParams.get("inbox_kind") === "personal";
      const tail = !!url.searchParams.get("continuation");
      if(!empty) { pages++; if(slow && pages > 1) await new Promise(resolve => setTimeout(resolve, 500)); }
      const selected = (empty ? [] : tail ? rows.slice(50) : rows.slice(0,50)).filter(m => !operations.some(o => o.target_id === m.id && o.change.kind === "archive"));
      return respond({ ...mail, rows: selected, continuation: !empty && !tail ? "last-page" : null, pending: !empty && !tail, coverage: [{ ...mail.coverage[0], state: incomplete && tail ? "partial" : "complete" }] });
    }
    if(p.endsWith("/conversation")) return respond({ rows: [rows[0]], complete: true, continuation: null });
    if(p.endsWith("/unsubscribe")) return respond(null);
    if(p.includes("/messages/")) return respond({ ...content, message: rows.find(m => p.endsWith(m.id)) || rows[0] });
    return respond([]);
  });
  await page.goto("/"); await page.getByRole("tab", { name: "Automated", exact: true }).click();
  await page.getByRole("button", { name: "Security alerts", exact: true }).click();
  await expect(page.locator('.message-row').first()).toBeVisible();
  return { rows, operations, pages: () => pages };
}
for (const width of [390,1480]) test(`visible selection expands beyond loaded pages and archives every match at ${width}`, async ({ page }, info) => {
  await page.setViewportSize({width,height:900}); const state = await fixture(page);
  await expect(page.getByRole("checkbox",{name:"Select Match 52",exact:true})).toHaveCount(0);
  const header = page.getByRole("checkbox",{name:"Select all visible messages",exact:true});
  await expect(header).toBeEnabled();
  await header.focus();await page.keyboard.press("Space");
  await expect.poll(() => page.locator(".message-select input:checked").count()).toBeGreaterThan(0);
  const visibleCount = await page.locator('.message-select input:checked').count();expect(visibleCount).toBeGreaterThan(0);expect(visibleCount).toBeLessThan(50);
  await page.getByRole("button",{name:"Select all messages matching this filter",exact:true}).focus();await page.keyboard.press("Enter");
  await expect(page.getByRole("status").filter({hasText:"All 53 matching messages are selected."})).toBeVisible();
  await page.screenshot({path:info.outputPath(`all-matches-${width}.png`),animations:"disabled"});
  await page.getByRole("button",{name:"Archive",exact:true}).click();
  await expect.poll(()=>state.operations.length).toBe(53);
  expect(new Set(state.operations.map(o=>o.target_id)).size).toBe(53);
  expect(state.operations.every(o=>o.account_id===accounts[0].id&&o.change.kind==="archive")).toBe(true);
  await expect(page.getByRole("status").filter({hasText:"53 of 53 messages updated."})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});
test("incomplete coverage leaves only the visible selection and never submits writes",async({page})=>{
  const state=await fixture(page,true);await page.getByRole("checkbox",{name:"Select all visible messages",exact:true}).check();
  const count=await page.locator('.message-select input:checked').count();
  await page.getByRole("button",{name:"Select all messages matching this filter",exact:true}).click();
  await expect(page.getByRole("alert")).toContainText("Couldn’t check every match");
  expect(await page.locator('.message-select input:checked').count()).toBe(count);expect(state.operations).toEqual([]);
});
test("changing filters cancels an in-flight all-selection",async({page})=>{
  const state=await fixture(page,false,true);await page.getByRole("checkbox",{name:"Select all visible messages",exact:true}).check();
  await page.getByRole("button",{name:"Select all messages matching this filter",exact:true}).click();
  await expect(page.getByRole("button",{name:"Cancel",exact:true})).toBeVisible();
  await page.getByRole("button",{name:"Flights",exact:true}).click();
  await expect(page.getByText(/All 53 matching messages/)).toHaveCount(0);
  await page.waitForTimeout(600);await expect(page.locator('.message-select input:checked')).toHaveCount(0);expect(state.operations).toEqual([]);
});
test("deselecting one match retains off-page targets for other bulk actions",async({page})=>{
  const state=await fixture(page);await page.getByRole("checkbox",{name:"Select all visible messages",exact:true}).check();
  await page.getByRole("button",{name:"Select all messages matching this filter",exact:true}).click();
  await expect(page.getByRole("button",{name:"Select visible only",exact:true})).toBeFocused();
  await page.getByRole("checkbox",{name:"Select Match 0",exact:true}).uncheck();
  await page.getByRole("button",{name:"Mark unread",exact:true}).click();
  await expect.poll(()=>state.operations.length).toBe(52);
  expect(state.operations.every(o=>o.change.kind==='read'&&o.change.value===false&&o.target_id!=='match-0')).toBe(true);
  for(const id of ['match-50','match-51','match-52'])expect(state.operations.some(o=>o.target_id===id)).toBe(true);
});
test("clear selection cancels expansion without restoring a late all-selection",async({page})=>{
  const state=await fixture(page,false,true);await page.getByRole("checkbox",{name:"Select all visible messages",exact:true}).check();
  await page.getByRole("button",{name:"Select all messages matching this filter",exact:true}).click();
  await page.getByRole('button',{name:/selected · Clear/}).click();
  await page.waitForTimeout(600);await expect(page.locator('.message-select input:checked')).toHaveCount(0);
  await expect(page.getByRole('button',{name:'Select visible only',exact:true})).toHaveCount(0);expect(state.operations).toEqual([]);
});
