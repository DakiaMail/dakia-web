import { test, expect, type Page } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";
import { classification } from "../src/classification";
import type { Message } from "@dakia/api-types";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const output = resolve("../../artifacts/quick-actions");
const reference = "SYNTHETIC-ORDER-LONG-REFERENCE-ONLY-FOR-LOCAL-TESTING";
const fixtures: Message[] = [
  ["Google", "Your account activity summary", "newsletter"],
  ["Temu", "Your order is on its way", "order"],
  ["The New York Times", "The Morning: A quieter inbox", "newsletter"],
  ["Google", "A few helpful updates for your account", "newsletter"],
  ["Temu", "Your weekly picks are ready", "newsletter"],
  ["A very long sender name for a narrow inbox layout", "A long subject with international text: 東京 · مرحباً · delivery update", "order"],
  ["Unknown reference", "No detected reference available", "order"],
  ["Cloud Harbour", "A receipt with a plain total", "receipt"],
  ["Final fixture", "End of the synthetic inbox", "newsletter"],
].map(([sender, subject, kind], i) => ({
  ...mail.rows[0], id: `context-${i}`, provider_id: `fixture-${i}`, thread_id: `context-${i}`,
  account_id: i === 6 ? "missing-account" : "personal", sender: { name: sender, email: `fixture-${i}@example.invalid` },
  subject, preview: "Synthetic fixture message. No live mail or private data.", unread: i === 1 || i === 2,
  received_at: `2026-11-${String(18-i).padStart(2,"0")}T08:42:00Z`,
  classification: { ...classification(mail.rows[0]), status: "ready", kind: "automated", types: kind === "order" ? ["order", "newsletter"] : kind === "receipt" ? ["receipt"] : ["newsletter"], attention: [],
    details: kind === "order" ? [{ email_type: "order", label: "Reference", value: i === 6 ? "" : reference, action: "copy" }, { email_type: "order", label: "Invoice reference", value: i === 6 ? "" : "SYNTHETIC-INVOICE", action: "copy" }] : kind === "receipt" ? [{ email_type: "receipt", label: "Total", value: "EUR 22.00", action: null }] : [] },
}));
async function fixture(page: Page, stubClipboard = true) {
  const writes: string[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    if (route.request().method() !== "GET") writes.push(path);
    const reply = (v: unknown) => route.fulfill({contentType:"application/json",body:JSON.stringify(v)});
    if(path === "/api/session") return reply({user_id:"contextual-fixture",csrf_token:"synthetic-local-only"});
    if(path === "/api/accounts") return reply([{...accounts[1],display_name:"Gmail Personal · Synthetic fixture account with a long name"}]);
    if(path.endsWith("/mailbox-counts")) return reply({starred:0,attention:0});
    if(path.endsWith("/thread-counts")) return reply({});
    if(path.endsWith("/sync-active")) return reply(false);
    if(path.endsWith("/operations") && route.request().method() === "POST") return reply({...route.request().postDataJSON(),status:"applied",revision:2});
    if(path.endsWith("/events")) return route.fulfill({contentType:"text/event-stream",body:": ready\n\n"});
    if(path === "/api/search" || path.endsWith("/messages")) return reply({...mail,rows:fixtures,continuation:null});
    const m = fixtures.find(m => path.includes(`/messages/${m.id}`)) || fixtures[0];
    if(path.endsWith("/conversation")) return reply({rows:[m],complete:true,continuation:null});
    if(path.endsWith("/unsubscribe")) return reply(null);
    if(path.includes("/messages/")) return reply({...content,message:m,html:"<p>Synthetic fixture message. No live mail.</p>",text:"Synthetic fixture message. No live mail.",attachments:[]});
    return reply([]);
  });
  await page.addInitScript((stubClipboard) => {
    localStorage.setItem("dakia-theme", "light");
    if (!stubClipboard) return;
    Object.defineProperty(navigator, "clipboard", { configurable:true, value: {writeText:async(value:string) => {
      const w=window as typeof window & {copiedValue?:string;denyCopy?:boolean};
      if(w.denyCopy) throw new Error("Synthetic clipboard permission denial");w.copiedValue=value;
    }}});
  }, stubClipboard);
  await page.goto("/");
  await page.getByRole("tab",{name:"Automated",exact:true}).click();
  await expect(page.locator(".message-row")).toHaveCount(fixtures.length);
  await page.evaluate(() => {document.title="Dakia · SYNTHETIC FIXTURE evidence"; const label=document.createElement("div"); label.textContent="SYNTHETIC FIXTURE · local UI review · no live mail";label.style.cssText="padding:5px 8px;background:#f2f1f7;color:#555;font:10px sans-serif;text-align:center;flex-shrink:0";document.querySelector(".mailbox")?.prepend(label);});
  return writes;
}
for(const width of [320,390,1480,2560]) test(`compact contextual actions at ${width}`,async({page})=>{
  await page.setViewportSize({width,height:width===1480?980:844});
  const writes=await fixture(page);
  const row=page.locator(".message-row").filter({hasText:"Your order is on its way"});
  const copy=row.getByRole("button",{name:"Copy order number",exact:true});
  await expect(copy).toBeVisible();
  if(width<601) await expect(row.locator(".mail-details").getByRole("button")).toHaveCount(1);
  const before=await copy.boundingBox();
  expect(before!.height).toBeGreaterThanOrEqual(width<601?44:28);
  await row.locator(".message-open").focus();await page.keyboard.press("Tab");
  await expect(copy).toBeFocused();await page.keyboard.press("Enter");
  await expect(copy).toHaveAttribute("data-copy-state","copied");
  expect(await page.evaluate(()=> (window as typeof window & {copiedValue?:string}).copiedValue)).toBe(reference);
  expect(await copy.boundingBox()).toEqual(before);
  await expect(page.locator(".conversation")).toHaveCount(0);
  await page.evaluate(()=>{(window as typeof window & {denyCopy?:boolean}).denyCopy=true;});
  await page.keyboard.press("Space");
  await expect(copy).toHaveAttribute("data-copy-state","error");
  await expect(row.getByRole("alert")).toContainText("Couldn’t copy");
  expect(await copy.boundingBox()).toEqual(before);
  await page.evaluate(()=>{(window as typeof window & {denyCopy?:boolean}).denyCopy=false;});
  if(width<601) await copy.tap(); else await copy.click();
  await expect(copy).toHaveAttribute("data-copy-state","copied");
  await expect(page.locator(".conversation")).toHaveCount(0);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  const account=await row.locator(".account-label").boundingBox();
  expect(account!.y+account!.height <= before!.y+1 || account!.x >= before!.x+before!.width).toBe(true);
  const receipt=page.locator(".message-row").filter({hasText:"A receipt with a plain total"});
  const total=(await receipt.getByText("Total: EUR 22.00",{exact:true}).boundingBox())!;
  const receiptAccount=(await receipt.locator(".account-label").boundingBox())!;
  expect(total.x+total.width<=receiptAccount.x || total.y+total.height<=receiptAccount.y || receiptAccount.y+receiptAccount.height<=total.y).toBe(true);
  await expect(page.locator(".message-row").filter({hasText:"No detected reference available"}).getByRole("button",{name:"Copy order number"})).toHaveCount(0);
  if(width<601) {
    const listBox=(await page.locator(".message-list").boundingBox())!;
    const composeBox=(await page.getByRole("button",{name:"Compose",exact:true}).boundingBox())!;
    expect(listBox.y+listBox.height).toBeLessThanOrEqual(composeBox.y);
  }
  mkdirSync(output,{recursive:true});
  await page.mouse.move(0,0);await copy.blur();
  await expect(copy).toHaveAttribute("data-copy-state","idle");
  await expect(copy.locator('[data-icon="copy"]')).toHaveCSS("opacity","1");
  await expect(copy.locator('[data-icon="copied"]')).toHaveCSS("opacity","0");
  await expect(row.locator(".account-label")).toHaveCSS("opacity","1");
  await page.screenshot({path:resolve(output,`fixture-inbox-${width}.png`)});
  await row.locator(".message-open").focus();await page.keyboard.press("Enter");
  await expect(page.locator(".conversation-body").getByRole("button",{name:"Copy invoice reference",exact:true})).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".message-row")).toHaveCount(fixtures.length);
  expect(writes.filter(p=>p.endsWith("/unsubscribe"))).toEqual([]);
  const last=page.locator(".message-row").last();await last.scrollIntoViewIfNeeded();
  const lastBox=await last.boundingBox();
  const compose=await page.getByRole("button",{name:"Compose",exact:true}).boundingBox();
  if(width<601 && compose && lastBox) expect(lastBox.y+lastBox.height).toBeLessThanOrEqual(compose.y);
  if(width===390) {
    await page.screenshot({path:resolve(output,"fixture-inbox-390-bottom.png")});
  }
  console.log(JSON.stringify({width,copyTarget:before,stableBounds:true,copySuccess:true,simulatedDenialAndRetry:true,readerSecondaryAction:true,noUnsubscribeWrites:true}));
});

test("category rail hides its scrollbar without losing keyboard, trackpad or touch scrolling", async ({page}) => {
  await page.setViewportSize({width:320,height:844});
  await fixture(page);
  const rail=page.locator(".email-type-filters");
  await expect(rail).toHaveCSS("scrollbar-width","none");
  await expect(rail).toHaveCSS("overflow-x","auto");
  await rail.getByRole("button").first().focus();
  for(let i=1;i<await rail.getByRole("button").count();i++) await page.keyboard.press("Tab");
  await expect(rail.getByRole("button").last()).toBeFocused();
  await expect.poll(()=>rail.evaluate(el=>el.scrollLeft)).toBeGreaterThan(0);
  await rail.evaluate(el=>{el.scrollLeft=0;});
  await rail.hover();await page.mouse.wheel(220,0);
  await expect.poll(()=>rail.evaluate(el=>el.scrollLeft)).toBeGreaterThan(0);
  await rail.evaluate(el=>{el.scrollLeft=0;});
  const box=(await rail.boundingBox())!;
  const cdp=await page.context().newCDPSession(page);
  await cdp.send("Input.dispatchTouchEvent",{type:"touchStart",touchPoints:[{x:280,y:box.y+box.height/2}]});
  for(const x of [240,200,160,120,80]) await cdp.send("Input.dispatchTouchEvent",{type:"touchMove",touchPoints:[{x,y:box.y+box.height/2}]});
  await cdp.send("Input.dispatchTouchEvent",{type:"touchEnd",touchPoints:[]});
  await expect.poll(()=>rail.evaluate(el=>el.scrollLeft)).toBeGreaterThan(0);
  await expect(page.locator(".message-list")).toHaveCSS("overflow-x","hidden");
  expect(await page.locator(".message-list").evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true);
});

test("real system Chromium clipboard and dark short viewport", async ({page,context}) => {
  await context.grantPermissions(["clipboard-read","clipboard-write"]);
  await page.setViewportSize({width:390,height:568});
  const writes=await fixture(page,false);
  const row=page.locator(".message-row").filter({hasText:"Your order is on its way"});
  const copy=row.getByRole("button",{name:"Copy order number",exact:true});
  await page.bringToFront();await copy.tap({position:{x:8,y:2}});
  await expect(copy).toHaveAttribute("data-copy-state","copied");
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(reference);
  await expect(page.locator(".conversation")).toHaveCount(0);
  await page.getByRole("button",{name:"Open navigation",exact:true}).click();
  await page.getByRole("button",{name:"Dark theme",exact:true}).click();
  await page.locator(".sidebar").getByRole("button",{name:"Close navigation",exact:true}).click();
  await expect(page.locator(".app")).not.toHaveClass(/nav-open/);
  await expect.poll(async()=> {const b=await page.locator(".sidebar").boundingBox();return b!.x+b!.width;}).toBeLessThanOrEqual(0);
  await expect(page.locator("html")).toHaveAttribute("data-theme","dark");
  await row.locator(".message-open").focus();await page.keyboard.press("Tab");
  await expect(copy).toBeFocused();await expect(copy).toHaveAttribute("data-copy-state","idle");
  await expect(copy.locator('[data-icon="copy"]')).toHaveCSS("opacity","1");
  await expect(copy.locator('[data-icon="copied"]')).toHaveCSS("opacity","0");
  await page.screenshot({path:resolve(output,"fixture-inbox-390-dark-short.png")});
  await page.keyboard.press("Enter");await expect(copy).toHaveAttribute("data-copy-state","copied");
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toBe(reference);
  expect(writes.filter(p=>p.endsWith("/unsubscribe"))).toEqual([]);
  console.log("Real system clipboard read-back matched the synthetic order reference; 44px target edge tap and keyboard activation passed in short dark viewport.");
});
