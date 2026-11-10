import { test, expect, type Page } from "@playwright/test";
import type { Message } from "@dakia/api-types";
const origin = "http://127.0.0.1:4346";
async function setup(page: Page) {
  await page.context().addCookies([{ name: "__Host-dakia", value: "translation-browser-fixture", url: "https://127.0.0.1:4346/", secure: true, sameSite: "Lax" }]);
  const rows: Message[] = await (await page.request.get("/__translation/messages")).json();
  const message = rows.find(m => m.id === "estonian-english-footer")!;
  await page.goto(`/?account=${message.account_id}&message=${message.id}`);
  await expect(page.getByRole("button", { name: "Translate to English", exact: true })).toBeVisible();
  return message;
}
async function policy(page: Page, mode: "device" | "automatic" | "host") {
  const response = await page.request.put("/api/translation/settings", { headers: { Origin: origin, "X-CSRF-Token": "translation-csrf" }, data: { mode } });
  expect(response.ok()).toBe(true);
  await page.reload();
}
async function keyboardActivate(page: Page, selector: string) {
  await expect(page.locator(selector)).toBeVisible();
  for (let i=0;i<350;i++) {
    if (await page.locator(selector).evaluate(node => node === document.activeElement)) { await page.keyboard.press("Enter"); return; }
    await page.keyboard.press("Tab");
  }
  throw new Error(`Keyboard could not reach ${selector}`);
}

test("real browser engine, verified saved packs, reload, original toggle and device-only enforcement", async ({ page }) => {
  const message = await setup(page);
  await policy(page,"device");
  const hostJobs: string[] = [];
  page.on("request", request => { if (request.method()==="POST" && new URL(request.url()).pathname.endsWith("/translation")) hostJobs.push(request.url()); });
  await keyboardActivate(page,'button[title="Translate to English"]');
  await expect(page.getByTitle("Translated message")).toBeVisible({timeout:150_000});
  expect(hostJobs).toEqual([]);
  await expect(page.frameLocator('iframe[title="Translated message"]').locator("body")).toContainText(/meeting|documents/i);
  const packs = await page.evaluate(async()=>{const cache=await caches.open("dakia-translation-packs-v1");return (await cache.keys()).map(r=>r.url);});
  expect(packs.some(url=>url.includes("/api/translation/models/"))).toBe(true);
  expect(packs.some(url=>url.endsWith(".wasm"))).toBe(true);
  await keyboardActivate(page,'button[title="Show original"]');
  await expect(page.getByTitle("Message body")).toBeVisible();
  await keyboardActivate(page,'button[title="Show translation"]');
  await expect(page.getByTitle("Translated message")).toBeVisible();
  await page.reload();
  // Cached packs must work even when every model/runtime network request fails.
  await page.route("**/api/translation/models/**", route=>route.abort());
  await page.route("**/*.wasm", route=>route.abort());
  // Let the authenticated message load before disconnecting. Offline startup
  // of the entire mail session is separate from saved-pack inference.
  await expect(page.getByRole("button", { name: "Translate to English", exact: true })).toBeVisible();
  await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined));
  await page.context().setOffline(true);
  await keyboardActivate(page,'button[title="Translate to English"]');
  await expect(page.getByTitle("Translated message")).toBeVisible();
  await page.context().setOffline(false);
  expect(hostJobs).toEqual([]);
  const denied=await page.request.post(`/api/accounts/${message.account_id}/messages/${message.id}/translation?language=et`,{headers:{Origin:origin,"X-CSRF-Token":"translation-csrf"}});
  expect(denied.status()).toBe(403);
  await keyboardActivate(page,'button[aria-label="Manage workspace"]');
  await expect(page.getByRole("button",{name:"Remove Estonian language"})).toBeVisible();
  await keyboardActivate(page,'button[aria-label="Remove Estonian language"]');
  await expect(page.getByText("Language removed from this browser.")).toBeVisible();
  await expect(page.getByRole("heading",{name:"Translation",exact:true})).toBeFocused();
  const remaining = await page.evaluate(async()=>{const c=await caches.open("dakia-translation-packs-v1");return (await c.keys()).filter(r=>r.url.includes("/api/translation/models/")).length;});
  expect(remaining).toBe(0);
});

test("administrator keyboard settings persist and focus in narrow/dark UI", async ({ page }) => {
  await setup(page);
  await policy(page,"host");
  await page.setViewportSize({width:390,height:560});
  await page.evaluate(()=>localStorage.setItem("dakia-theme","dark")); await page.goto("/");
  // Narrow navigation is a separate keyboard surface.
  const nav = page.getByRole("button",{name:"Open navigation",exact:true});
  await keyboardActivate(page,'button[aria-label="Open navigation"]');
  await keyboardActivate(page,'button[aria-label="Manage workspace"]');
  await expect(page.getByRole("heading",{name:"Translation",exact:true})).toBeVisible();
  const local=page.getByRole("radio",{name:"On devices Translate on each person’s device. No host translation.",exact:true});
  await local.focus(); await page.keyboard.press("Space");
  await keyboardActivate(page,'button:has-text("Save translation settings")');
  await expect(page.getByText("Translation settings saved.",{exact:true})).toBeVisible();
  await page.goto("/");
  await keyboardActivate(page,'button[aria-label="Open navigation"]');
  await keyboardActivate(page,'button[aria-label="Manage workspace"]');
  await expect(page.getByRole("radio",{name:/^On devices Translate on/})).toBeChecked();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth)).toBe(false);
});

test("unsupported device uses real host fallback only in automatic mode", async ({ page }) => {
  await page.addInitScript(()=>Object.defineProperty(window,"Worker",{value:undefined,configurable:true}));
  await setup(page); await policy(page,"automatic");
  await keyboardActivate(page,'button[title="Translate to English"]');
  await expect(page.getByText("Using host translation because this device could not finish.")).toBeVisible();
  await expect(page.getByTitle("Translated message")).toBeVisible({timeout:150_000});
  await policy(page,"device");
  await keyboardActivate(page,'button[title="Translate to English"]');
  await expect(page.getByText(/Translation is not supported on this device/)).toBeVisible();
});

for (const width of [390, 1480]) for (const dark of [false,true]) test(`download circle and keyboard cancellation ${width} ${dark ? "dark" : "light"}`, async ({page})=>{
  await page.setViewportSize({width,height:560});
  await page.addInitScript(dark=>localStorage.setItem("dakia-theme",dark?"dark":"light"),dark);
  await setup(page);await policy(page,"device");
  await expect(page.getByRole("button",{name:"Translate to English",exact:true})).toHaveAttribute("aria-disabled","false");
  const session=await page.context().newCDPSession(page);
  await session.send("Network.enable");
  await session.send("Network.emulateNetworkConditions",{offline:false,latency:20,downloadThroughput:300000,uploadThroughput:200000});
  const hostJobs:string[]=[];
  page.on("request",r=>{if(r.method()==="POST"&&new URL(r.url()).pathname.endsWith("/translation"))hostJobs.push(r.url());});
  await keyboardActivate(page,'button[title="Translate to English"]');
  const ring=page.getByRole("progressbar",{name:"Downloading Estonian"});
  await expect(ring).toBeVisible();
  await expect.poll(async()=>Number(await ring.getAttribute("aria-valuenow"))).toBeGreaterThan(0);
  const value=Number(await ring.getAttribute("aria-valuenow"));expect(value).toBeLessThan(100);
  const {mkdirSync}=await import("node:fs");mkdirSync("test-results/device-translation/visual",{recursive:true});
  await page.screenshot({path:`test-results/device-translation/visual/download-${width}-${dark?"dark":"light"}.png`});
  await keyboardActivate(page,'button[aria-label="Cancel translation"]');
  await expect(ring).toHaveCount(0);
  await expect(page.getByRole("button",{name:"Translate to English",exact:true})).toBeFocused();
  expect(hostJobs).toEqual([]);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth)).toBe(false);
  await session.send("Network.emulateNetworkConditions",{offline:false,latency:0,downloadThroughput:-1,uploadThroughput:-1});
});

test("corrupt saved weights are replaced before local inference", async({page})=>{
  await setup(page);await policy(page,"device");
  await keyboardActivate(page,'button[title="Translate to English"]');
  await expect(page.getByTitle("Translated message")).toBeVisible({timeout:120000});
  const hash="df844b3a6df906b2e0682fb7765694f35b351854db00f964896d9c8c365fbff3";
  await page.evaluate(async hash=>{const cache=await caches.open("dakia-translation-packs-v1");await cache.put(`/api/translation/models/${hash}`,new Response("corrupt public weight"));},hash);
  await page.reload();
  const downloaded:string[]=[];
  page.on("request",r=>{if(new URL(r.url()).pathname===`/api/translation/models/${hash}`)downloaded.push(r.url());});
  await keyboardActivate(page,'button[title="Translate to English"]');
  await expect(page.getByTitle("Translated message")).toBeVisible();
  expect(downloaded).toHaveLength(1);
  const bytes=await page.evaluate(async hash=>{const cache=await caches.open("dakia-translation-packs-v1");return (await (await cache.match(`/api/translation/models/${hash}`))!.arrayBuffer()).byteLength;},hash);
  expect(bytes).toBe(31561787);
});
